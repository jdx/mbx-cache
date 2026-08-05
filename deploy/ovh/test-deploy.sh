#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/mise-cache-deploy-test.XXXXXXXX")

cleanup() {
  local status=$?
  case "$test_root" in
    "${TMPDIR:-/tmp}"/mise-cache-deploy-test.*) rm -rf -- "$test_root" ;;
    *) echo "refusing to remove unexpected test directory: $test_root" >&2 ;;
  esac
  return "$status"
}
trap cleanup EXIT

install -d "$test_root/bin" "$test_root/capture"

cat >"$test_root/bin/fake-terraform" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case "${*: -1}" in
  server_ipv4) printf '%s\n' '192.0.2.10' ;;
  cache_url) printf '%s\n' 'https://cache.example.com' ;;
  r2_bucket) printf '%s\n' 'mise-cache-production' ;;
  r2_endpoint) printf '%s\n' 'https://0123456789abcdef.r2.cloudflarestorage.com' ;;
  *) echo "unexpected Terraform output: ${*: -1}" >&2; exit 1 ;;
esac
SH

cat >"$test_root/bin/mise" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ $* == 'bootstrap remote --help' ]]; then
  exit 0
fi

printf '%s\n' "$@" >"$CAPTURE_DIR/args"
project_dir=
previous=
for argument in "$@"; do
  if [[ $previous == --source ]]; then
    project_dir=$argument
    break
  fi
  previous=$argument
done
[[ -n $project_dir && -d $project_dir ]]
printf '%s\n' "$project_dir" >"$CAPTURE_DIR/project-dir"
[[ $(stat -c %a "$project_dir/runtime") == 700 ]]
[[ $(stat -c %a "$project_dir/runtime/.env") == 600 ]]
[[ $(stat -c %a "$project_dir/runtime/cache.env") == 600 ]]
grep -Fq 'POSTGRES_PASSWORD="database_password_123456"' "$project_dir/runtime/.env"
grep -Fq 'AWS_ACCESS_KEY_ID="r2-access-key"' "$project_dir/runtime/cache.env"
grep -Fq 'AWS_SECRET_ACCESS_KEY="r2-secret-key"' "$project_dir/runtime/cache.env"
grep -Fq 'port = 2222' "$project_dir/mise.local.toml"
grep -Fq 'source = "203.0.113.10/32"' "$project_dir/mise.local.toml"
if grep -R -Fq 'must-not-be-forwarded' "$project_dir"; then
  echo "unrelated caller environment was copied" >&2
  exit 1
fi
SH
chmod 0755 "$test_root/bin/fake-terraform" "$test_root/bin/mise"

common_env=(
  "CAPTURE_DIR=$test_root/capture"
  "MISE_CACHE_DATABASE_PASSWORD=database_password_123456"
  "MISE_CACHE_IMAGE=ghcr.io/jdx/mise-cache@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  "PATH=$test_root/bin:$PATH"
  "R2_ACCESS_KEY_ID=r2-access-key"
  "R2_SECRET_ACCESS_KEY=r2-secret-key"
  "SHOULD_NOT_COPY=must-not-be-forwarded"
  "TERRAFORM_COMMAND=fake-terraform"
)

if env "${common_env[@]}" "$script_dir/deploy.sh" --dry-run >/dev/null 2>&1; then
  echo "deploy.sh accepted a missing OVH_SSH_SOURCE_CIDR" >&2
  exit 1
fi

if env "${common_env[@]}" \
  MISE_CACHE_IMAGE=ghcr.io/jdx/mise-cache:0.1.0 \
  OVH_SSH_SOURCE_CIDR=203.0.113.10/32 \
  "$script_dir/deploy.sh" --dry-run >/dev/null 2>&1; then
  echo "deploy.sh accepted an image that was not pinned by digest" >&2
  exit 1
fi

env "${common_env[@]}" \
  OVH_SSH_PORT=2222 \
  OVH_SSH_SOURCE_CIDR=203.0.113.10/32 \
  "$script_dir/deploy.sh" --dry-run

grep -Fxq 'bootstrap' "$test_root/capture/args"
grep -Fxq 'remote' "$test_root/capture/args"
grep -Fxq 'ubuntu@192.0.2.10' "$test_root/capture/args"
grep -Fxq 'packages,files,services,firewall,compose' "$test_root/capture/args"
grep -Fxq -- '--port' "$test_root/capture/args"
grep -Fxq '2222' "$test_root/capture/args"
grep -Fq -- '--dry-run' "$test_root/capture/args"
if grep -Eq 'database_password|r2-(access|secret)-key' "$test_root/capture/args"; then
  echo "secret value appeared in mise arguments" >&2
  exit 1
fi

project_dir=$(<"$test_root/capture/project-dir")
if [[ -e $project_dir ]]; then
  echo "temporary bootstrap project was not removed: $project_dir" >&2
  exit 1
fi
