# OVH US deployment

This deployment runs Caddy, mise-cache, and PostgreSQL on one OVHcloud VPS in
Vint Hill, Virginia. Cache blobs live in a Cloudflare R2 bucket with an Eastern
North America location hint. The server is disposable; PostgreSQL is small and
the much larger blob store remains outside the server.

Terraform provisions:

- one monthly OVH US VPS in `US-EAST-VA`;
- an R2 Standard bucket with the `enam` location hint; and
- a DNS-only Cloudflare record pointing at the VPS.

[`mise bootstrap remote`](https://mise.jdx.dev/bootstrap/remote.html) installs
and converges the host firewall, fail2ban, automatic security updates, Docker,
Caddy, PostgreSQL, and mise-cache. Runtime secrets are copied only through a
protected temporary bootstrap project; they do not enter Terraform state, mise
configuration, Git, or OVH installation metadata.

## Cost profile

The entry OVH VPS currently starts around USD 4.54/month and includes an IPv4
address, daily backup, and unlimited traffic. R2 Standard storage is USD
0.015/GB-month with 10 GB-month included and free egress. Approximate
VPS-plus-storage totals are USD 5.89/month for 100 GB or USD 8.14/month for
250 GB; R2 operation charges can add to these estimates. Check current prices
and plan availability before applying the configuration.

## Prerequisites

- Terraform or OpenTofu 1.8 or newer
- mise 2026.8.2 or newer
- local `curl`, `jq`, `ssh`, and `tar` commands
- an OVH US account with a default payment method
- OVH API credentials authorized to order and manage a VPS
- a current OVH US VPS plan code and Ubuntu image ID
- a Cloudflare API token with R2 bucket and DNS edit permissions
- a separate R2 S3 token restricted to the cache bucket
- a published, immutable mise-cache image tag or digest

Set `TERRAFORM_COMMAND=tofu` when using OpenTofu for the deploy step.

OVH plan codes and availability change. Query the OVH US order catalog rather
than copying a stale value:

1. Create and assign a cart through the OVH API.
2. Call `GET /order/cart/{cartId}/vps` and select the current VPS-1 plan.
3. Use `US-EAST-VA` for `vps_datacenter` and `Ubuntu 24.04` for `vps_os`.

The `ovh_vps` Terraform resource requires an image ID when installing a public
SSH key. Retrieve the current Ubuntu image ID from
`GET /vps/{serviceName}/images/available` on an existing OVH VPS. This is an
OVH API limitation; the ID is then declaratively pinned for installation.

## Provision infrastructure

Copy the example variables file and edit its values:

```sh
cd deploy/ovh/terraform
cp terraform.tfvars.example terraform.tfvars
```

Export provider credentials, then review and apply the order carefully because
creating `ovh_vps.cache` purchases a recurring service:

```sh
export OVH_ENDPOINT=ovh-us
export OVH_APPLICATION_KEY=...
export OVH_APPLICATION_SECRET=...
export OVH_CONSUMER_KEY=...
export CLOUDFLARE_API_TOKEN=...
terraform init
terraform plan
terraform apply
```

Use a remote encrypted Terraform backend for production state. Local state and
`terraform.tfvars` are ignored by Git.

The Cloudflare DNS record deliberately has proxying disabled. Cache blobs may
be larger than Cloudflare's proxied request-body limits, so requests go directly
to Caddy on the VPS.

## Create scoped R2 credentials

After Terraform creates the bucket, create an R2 API token with Object Read &
Write permission scoped only to that bucket. Save its Access Key ID and Secret
Access Key; Cloudflare displays the secret only once. The general Cloudflare API
token used by Terraform and the S3 token used by mise-cache are intentionally
separate.

## Deploy the service

Pin `MISE_CACHE_IMAGE` by digest so a deployment cannot silently select changed
image content:

```sh
export MISE_CACHE_IMAGE=ghcr.io/jdx/mise-cache@sha256:<64-hex-digit-digest>
export MISE_CACHE_DATABASE_PASSWORD="$(openssl rand -hex 24)"
export R2_ACCESS_KEY_ID=...
export R2_SECRET_ACCESS_KEY=...
export OVH_SSH_SOURCE_CIDR="203.0.113.10/32"
./deploy/ovh/deploy.sh
```

`OVH_SSH_SOURCE_CIDR` is required: there is no world-open default. Mise also
checks the active SSH connection against this rule and `OVH_SSH_PORT` before
atomically applying the nftables policy, so an incorrect CIDR or port fails
before it can lock out the operator. The declared policy permits TCP 80/443,
UDP 443 for HTTP/3, and SSH only from the supplied CIDR.

The deploy script reads the hostname, server address, R2 endpoint, and bucket
from Terraform outputs. It creates a mode-`0700` local bootstrap project,
writes only the explicitly required runtime values into mode-`0600` source
files, and runs `mise bootstrap remote`. Mise stages the project in another
mode-`0700` temporary directory on the host, converges the protected service
environment files, and removes the staging directory. A shell trap removes the
local project on success or failure. The caller's other environment variables
are never forwarded.

Use [fnox](https://fnox.jdx.dev/) or another secret manager to populate the
explicit deployment environment without storing values in shell history:

```sh
fnox exec -- ./deploy/ovh/deploy.sh
```

Ubuntu images use the `ubuntu` SSH user by default. Override `OVH_SSH_USER`,
`OVH_SSH_PORT`, or `OVH_SSH_IDENTITY_FILE` when necessary; normal OpenSSH
configuration and host-key policy still apply. Additional arguments are passed
to `mise bootstrap remote`, so the complete remote plan can be inspected
without changing the host:

```sh
./deploy/ovh/deploy.sh --dry-run
```

On apply, the wrapper updates package metadata, converges only the package,
file, service, firewall, and Compose phases, then waits up to ten minutes for
the public HTTPS status endpoint. Keep the database password in a password
manager because changing it after PostgreSQL initializes requires a database
role-password rotation.

GitHub OIDC is configured with these server-enforced grants:

- `jdx/mise` on `main`: read/write;
- `jdx/mise` tag workflows: read/write;
- pull-request workflows: read-only; and
- other push workflows: read-only.

Override `MISE_CACHE_GITHUB_REPOSITORY` and `MISE_CACHE_GITHUB_OWNER_ID` when
deploying for another repository owner.

## Verify and operate

```sh
curl --fail "$(terraform -chdir=deploy/ovh/terraform output -raw cache_url)/v1/status"
ssh ubuntu@"$(terraform -chdir=deploy/ovh/terraform output -raw server_ipv4)" \
  'cd /opt/mise-cache && sudo docker compose ps'
```

The desired host state lives in `deploy/ovh/bootstrap/mise.toml`. Re-running
the deployment is convergent: mise skips matching packages and files, verifies
service state and the atomic firewall fingerprint, and recreates Compose
containers only when their effective configuration has changed.

Prometheus metrics remain available to containers at `/metrics`, but Caddy
does not expose that endpoint publicly. Add a private metrics collector before
enabling external monitoring.

Do not add an R2 expiration lifecycle yet. The current metadata store does not
garbage-collect references when objects expire, so independent R2 expiration
can leave action results pointing at missing blobs. Monitor usage while
coordinated metadata/blob garbage collection is implemented.
