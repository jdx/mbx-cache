#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

terraform_command="${TERRAFORM_COMMAND:-terraform}"

for command in ansible-playbook "$terraform_command"; do
  if ! command -v "$command" >/dev/null; then
    echo "required command not found: $command" >&2
    exit 1
  fi
done

server_ip="$("$terraform_command" -chdir=terraform output -raw server_ipv4)"
export MISE_CACHE_DOMAIN="${MISE_CACHE_DOMAIN:-$("$terraform_command" -chdir=terraform output -raw cache_url | sed 's#^https://##')}"
export HETZNER_OBJECT_STORAGE_BUCKET="${HETZNER_OBJECT_STORAGE_BUCKET:-$("$terraform_command" -chdir=terraform output -raw object_storage_bucket)}"
export HETZNER_OBJECT_STORAGE_LOCATION="${HETZNER_OBJECT_STORAGE_LOCATION:-nbg1}"

if [[ -z "${MISE_CACHE_DATABASE_PASSWORD:-}" ]]; then
  echo "MISE_CACHE_DATABASE_PASSWORD must be set" >&2
  exit 1
fi

ansible-playbook \
  --inventory "${server_ip}," \
  --user root \
  ansible/playbook.yml
