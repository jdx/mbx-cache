# Hetzner deployment

This deployment runs Caddy, mise-cache, and PostgreSQL on one Hetzner Cloud
server. Cache blobs live in Hetzner Object Storage in the same location. The
server is disposable; its small PostgreSQL database is protected by Hetzner
server backups, while the much larger blob store remains outside the server.

Terraform provisions:

- one Ubuntu 24.04 CX23 server in Nuremberg by default;
- a firewall exposing only HTTP, HTTPS, ICMP, and SSH from configured CIDRs;
- an S3-compatible Object Storage bucket;
- a DNS-only Cloudflare record; and
- Hetzner's rotating server backups.

Ansible installs and converges the Compose deployment without putting runtime
secrets into Terraform state or cloud-init metadata. Caddy obtains and renews
TLS certificates. PostgreSQL and mise-cache are reachable only on the private
Compose network.

## Cost profile

The default server and an IPv4 address cost approximately EUR 5.99/month before
tax. Hetzner Object Storage has a EUR 4.99/month base price that includes 1 TB
of storage and 1 TB of public egress. Traffic between Object Storage and the
server in `eu-central` is free, and the server includes 20 TB of outgoing
traffic. Server backups add 20 percent of the server price. Check current
Hetzner pricing before applying the configuration.

## Prerequisites

- Terraform or OpenTofu 1.8 or newer
- Ansible
- a Hetzner Cloud project, API token, and registered SSH key
- Hetzner Object Storage S3 credentials
- a Cloudflare API token with DNS edit access and the zone ID
- a published, immutable mise-cache image tag or digest

Set `TERRAFORM_COMMAND=tofu` when using OpenTofu for the deploy step.

Hetzner Object Storage credentials are created in the Hetzner Console. Use a
dedicated project and key pair for this deployment.

## Provision infrastructure

Copy the example variables file and edit only non-secret values:

```sh
cd deploy/hetzner/terraform
cp terraform.tfvars.example terraform.tfvars
```

Export provider credentials and the secret Terraform inputs:

```sh
export HCLOUD_TOKEN=...
export CLOUDFLARE_API_TOKEN=...
export TF_VAR_object_storage_access_key=...
export TF_VAR_object_storage_secret_key=...
terraform init
terraform plan
terraform apply
```

Use a remote encrypted Terraform backend for the production state. The local
state files and `terraform.tfvars` are ignored by Git.

The Cloudflare record deliberately has proxying disabled. Cache blobs may be
larger than Cloudflare's proxied request-body limits, so traffic must go directly
to Caddy on the server.

## Deploy the service

Use a version tag or digest for `MISE_CACHE_IMAGE`; do not deploy `latest`:

```sh
export MISE_CACHE_IMAGE=ghcr.io/jdx/mise-cache:0.1.0
export MISE_CACHE_DATABASE_PASSWORD="$(openssl rand -hex 24)"
export HETZNER_OBJECT_STORAGE_ACCESS_KEY="$TF_VAR_object_storage_access_key"
export HETZNER_OBJECT_STORAGE_SECRET_KEY="$TF_VAR_object_storage_secret_key"
export HETZNER_OBJECT_STORAGE_LOCATION=nbg1
./deploy/hetzner/deploy.sh
```

Keep `MISE_CACHE_DATABASE_PASSWORD` in a password manager. Changing it after
PostgreSQL initializes requires rotating the database role password separately.

The deploy script reads the hostname, server IP, and bucket from Terraform
outputs. It configures GitHub OIDC with these server-enforced grants:

- `jdx/mise` on `main`: read/write;
- `jdx/mise` tag workflows: read/write;
- pull-request workflows: read-only; and
- other push workflows: read-only.

Override `MISE_CACHE_GITHUB_REPOSITORY` and `MISE_CACHE_GITHUB_OWNER_ID` when
deploying for another repository owner.

## Verify and operate

```sh
curl --fail "$(terraform -chdir=deploy/hetzner/terraform output -raw cache_url)/v1/status"
ssh root@"$(terraform -chdir=deploy/hetzner/terraform output -raw server_ipv4)" \
  'cd /opt/mise-cache && docker compose ps'
```

Prometheus metrics remain available to containers at `/metrics`, but Caddy
does not expose that endpoint publicly. Add a private metrics collector before
enabling external monitoring.

Do not add an Object Storage expiration lifecycle yet. The current metadata
store does not garbage-collect references when objects expire, so independent
S3 expiration can leave action results pointing at missing blobs. The included
1 TB provides room to measure real usage while coordinated metadata/blob
garbage collection is implemented.
