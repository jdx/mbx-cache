#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
terraform_dir="$script_dir/terraform"
terraform_command="${TERRAFORM_COMMAND:-terraform}"

for command in curl jq mise "$terraform_command"; do
  if ! command -v "$command" >/dev/null; then
    echo "required command not found: $command" >&2
    exit 1
  fi
done

if ! mise bootstrap remote --help >/dev/null 2>&1; then
  echo "mise 2026.8.2 or newer with 'bootstrap remote' is required" >&2
  exit 1
fi

require_env() {
  local name=$1
  if [[ -z ${!name-} ]]; then
    echo "$name must be set" >&2
    exit 1
  fi
}

for name in \
  MISE_CACHE_DATABASE_PASSWORD \
  MISE_CACHE_IMAGE \
  OVH_SSH_SOURCE_CIDR \
  R2_ACCESS_KEY_ID \
  R2_SECRET_ACCESS_KEY; do
  require_env "$name"
done

if [[ ! $MISE_CACHE_DATABASE_PASSWORD =~ ^[A-Za-z0-9_-]{24,}$ ]]; then
  echo "MISE_CACHE_DATABASE_PASSWORD must contain at least 24 URL-safe characters" >&2
  exit 1
fi
if [[ $MISE_CACHE_IMAGE == *:latest ]]; then
  echo "MISE_CACHE_IMAGE must use an immutable version tag or digest, not latest" >&2
  exit 1
fi
if [[ $OVH_SSH_SOURCE_CIDR =~ [[:space:]] ]]; then
  echo "OVH_SSH_SOURCE_CIDR must be one CIDR without whitespace" >&2
  exit 1
fi

github_repository=${MISE_CACHE_GITHUB_REPOSITORY:-jdx/mise}
github_owner_id=${MISE_CACHE_GITHUB_OWNER_ID:-216188}
ssh_user=${OVH_SSH_USER:-ubuntu}
ssh_port=${OVH_SSH_PORT:-22}

if [[ ! $github_repository =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "MISE_CACHE_GITHUB_REPOSITORY must be an owner/repository name" >&2
  exit 1
fi
if [[ ! $github_owner_id =~ ^[0-9]+$ ]]; then
  echo "MISE_CACHE_GITHUB_OWNER_ID must be numeric" >&2
  exit 1
fi
if [[ ! $ssh_user =~ ^[A-Za-z_][A-Za-z0-9_-]*$ ]]; then
  echo "OVH_SSH_USER is not a valid SSH user name" >&2
  exit 1
fi
if [[ ! $ssh_port =~ ^[0-9]+$ ]] || ((ssh_port < 1 || ssh_port > 65535)); then
  echo "OVH_SSH_PORT must be between 1 and 65535" >&2
  exit 1
fi
if [[ -n ${OVH_SSH_IDENTITY_FILE:-} && ! -r $OVH_SSH_IDENTITY_FILE ]]; then
  echo "OVH_SSH_IDENTITY_FILE is not readable: $OVH_SSH_IDENTITY_FILE" >&2
  exit 1
fi

server_ip="$("$terraform_command" -chdir="$terraform_dir" output -raw server_ipv4)"
cache_url="$("$terraform_command" -chdir="$terraform_dir" output -raw cache_url)"
r2_bucket="$("$terraform_command" -chdir="$terraform_dir" output -raw r2_bucket)"
r2_endpoint="$("$terraform_command" -chdir="$terraform_dir" output -raw r2_endpoint)"
cache_url=${cache_url%/}

if [[ $cache_url != https://* ]]; then
  echo "Terraform cache_url must use https://" >&2
  exit 1
fi
cache_domain=${cache_url#https://}
if [[ ! $cache_domain =~ ^[A-Za-z0-9.-]+$ ]]; then
  echo "Terraform cache_url must contain a DNS hostname without a path" >&2
  exit 1
fi
if [[ ! $r2_endpoint =~ ^https://[a-f0-9]+[.]r2[.]cloudflarestorage[.]com$ ]]; then
  echo "Terraform r2_endpoint is not a Cloudflare R2 endpoint" >&2
  exit 1
fi

oidc_providers=$(jq -cn \
  --arg audience "$cache_url" \
  --arg owner_id "$github_owner_id" \
  --arg repository "$github_repository" \
  '[{
    issuer: "https://token.actions.githubusercontent.com",
    audiences: [$audience],
    rules: [
      {claims: {repository: $repository, repository_owner_id: $owner_id, ref: "refs/heads/main"}, read: [$repository], write: [$repository]},
      {claims: {repository: $repository, repository_owner_id: $owner_id, ref_type: "tag"}, read: [$repository], write: [$repository]},
      {claims: {repository: $repository, repository_owner_id: $owner_id, event_name: "pull_request"}, read: [$repository], write: []},
      {claims: {repository: $repository, repository_owner_id: $owner_id, event_name: "push"}, read: [$repository], write: []}
    ]
  }]')

temporary_root=${TMPDIR:-/tmp}
temporary_root=${temporary_root%/}
project_dir=$(mktemp -d "$temporary_root/mise-cache-ovh.XXXXXXXX")

cleanup() {
  local status=$?
  case "$project_dir" in
    "$temporary_root"/mise-cache-ovh.*) rm -rf -- "$project_dir" ;;
    *) echo "refusing to remove unexpected temporary directory: $project_dir" >&2 ;;
  esac
  return "$status"
}
trap cleanup EXIT

cp -R "$script_dir/bootstrap/." "$project_dir/"
install -d -m 0700 "$project_dir/runtime"

write_dotenv() {
  local encoded key=$1 value=$2
  encoded=$(jq -Rn --arg value "$value" '$value')
  printf '%s=%s\n' "$key" "$encoded"
}

{
  write_dotenv MISE_CACHE_DOMAIN "$cache_domain"
  write_dotenv MISE_CACHE_IMAGE "$MISE_CACHE_IMAGE"
  write_dotenv POSTGRES_PASSWORD "$MISE_CACHE_DATABASE_PASSWORD"
} >"$project_dir/runtime/.env"

{
  write_dotenv MISE_CACHE_STORAGE s3
  write_dotenv MISE_CACHE_DATABASE_URL \
    "postgres://mise_cache:$MISE_CACHE_DATABASE_PASSWORD@postgres/mise_cache"
  write_dotenv MISE_CACHE_S3_BUCKET "$r2_bucket"
  write_dotenv MISE_CACHE_S3_PREFIX v1
  write_dotenv MISE_CACHE_S3_ENDPOINT "$r2_endpoint"
  write_dotenv MISE_CACHE_S3_REGION auto
  write_dotenv MISE_CACHE_S3_PATH_STYLE true
  write_dotenv MISE_CACHE_OIDC_PROVIDERS_JSON "$oidc_providers"
  write_dotenv MISE_CACHE_ALLOW_ANONYMOUS false
  write_dotenv AWS_ACCESS_KEY_ID "$R2_ACCESS_KEY_ID"
  write_dotenv AWS_SECRET_ACCESS_KEY "$R2_SECRET_ACCESS_KEY"
} >"$project_dir/runtime/cache.env"
chmod 0600 "$project_dir/runtime/.env" "$project_dir/runtime/cache.env"

ssh_source_cidr=$(jq -Rn --arg value "$OVH_SSH_SOURCE_CIDR" '$value')
{
  printf '%s\n' '[[bootstrap.linux.firewall.rules]]'
  printf '%s\n' 'name = "ssh-admin"'
  printf '%s\n' 'direction = "incoming"'
  printf '%s\n' 'action = "allow"'
  printf 'port = %s\n' "$ssh_port"
  printf '%s\n' 'protocol = "tcp"'
  printf 'source = %s\n' "$ssh_source_cidr"
} >"$project_dir/mise.local.toml"

remote_args=(
  bootstrap remote
  --host "$ssh_user@$server_ip"
  --source "$project_dir"
  --only "packages,files,services,firewall,compose"
  --update
  --yes
)
if [[ $ssh_port != 22 ]]; then
  remote_args+=(--port "$ssh_port")
fi
if [[ -n ${OVH_SSH_IDENTITY_FILE:-} ]]; then
  remote_args+=(--identity-file "$OVH_SSH_IDENTITY_FILE")
fi

dry_run=false
for argument in "$@"; do
  if [[ $argument == --dry-run || $argument == -n ]]; then
    dry_run=true
  fi
done

mise "${remote_args[@]}" "$@"

if [[ $dry_run == false ]]; then
  curl \
    --connect-timeout 10 \
    --fail \
    --retry 60 \
    --retry-all-errors \
    --retry-delay 5 \
    --retry-max-time 600 \
    --show-error \
    --silent \
    "$cache_url/v1/status" >/dev/null
  echo "mise-cache is healthy at $cache_url"
fi
