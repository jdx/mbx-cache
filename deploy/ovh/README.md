# OVH US deployment

This deployment runs Caddy, mise-cache, and PostgreSQL on one OVHcloud VPS in
Vint Hill, Virginia. Cache blobs live in a manually created Cloudflare R2
bucket. The server is disposable; PostgreSQL is small and the much larger blob
store remains outside the server.

Terraform records or provisions the server:

- infrastructure around an existing VPS identified by IPv4 address, or one new
  monthly OVH US VPS in `US-EAST-VA`.

The R2 bucket and DNS record are deliberately created in Cloudflare by an
operator. Terraform therefore needs no Cloudflare API token, and mise-cache can
use an Object Read & Write token scoped to only its bucket.

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
- an existing Ubuntu VPS with key-based SSH access, or an OVH US account with
  credentials and a default payment method for ordering a new VPS
- when `OVH_SSH_HOST` uses a tailnet address, a server already enrolled in the
  same tailnet with TCP 22 permitted by its tailnet policy, plus a connected
  local Tailscale client and `tailscale` command; bootstrap permits SSH on the
  existing `tailscale0` interface but does not install or enroll Tailscale
- a current OVH US VPS plan code and Ubuntu image ID only when ordering a VPS
- an existing R2 bucket and Object Read & Write token restricted to that bucket
- a DNS-only Cloudflare A record for the public cache hostname
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

Set `existing_server_ipv4` to adopt a server that has already been purchased.
In that mode Terraform does not create or manage the VPS, and the OVH provider
does not need credentials. Leave it unset and provide `plan_code`, `image_id`,
and `public_ssh_key` to order a new server.

Do not switch a VPS that this configuration already manages directly into
adoption mode. The resource has `prevent_destroy` enabled, so Terraform rejects
the otherwise destructive transition. First record and verify
`terraform output -raw server_ipv4`, set `existing_server_ipv4` to that exact
address, remove only `ovh_vps.cache[0]` from state with
`terraform state rm 'ovh_vps.cache[0]'`, and then review a fresh plan before
applying. Removing the resource from state does not delete the VPS.

Export OVH provider credentials only when ordering a VPS, then review the plan
carefully.
Creating `ovh_vps.cache[0]` purchases a recurring service; an existing-server
plan must not contain that resource:

```sh
export OVH_ENDPOINT=ovh-us
export OVH_APPLICATION_KEY=...
export OVH_APPLICATION_SECRET=...
export OVH_CONSUMER_KEY=...
terraform init
terraform plan
terraform apply
```

Use a remote encrypted Terraform backend for production state. Local state and
`terraform.tfvars` are ignored by Git.

For an existing server, the Terraform state contains configuration outputs but
no managed infrastructure resources.

## Create R2 storage and DNS

Create the `mise-cache-production` R2 bucket with Standard storage in Eastern
North America, then create an R2 API token with Object Read & Write permission
scoped only to that bucket. Save its Access Key ID and Secret Access Key;
Cloudflare displays the secret only once.

Create a DNS-only A record from `cache.mise.jdx.dev` to the Terraform
`server_ipv4` output. Do not enable the Cloudflare proxy: cache blobs may be
larger than proxied request-body limits, so requests must go directly to Caddy
on the VPS.

## Deploy the service

Pin `MISE_CACHE_IMAGE` by digest so a deployment cannot silently select changed
image content:

```sh
export MISE_CACHE_IMAGE=ghcr.io/jdx/mise-cache@sha256:<64-hex-digit-digest>
export MISE_CACHE_DATABASE_PASSWORD="$(openssl rand -hex 24)"
export R2_ACCESS_KEY_ID=...
export R2_SECRET_ACCESS_KEY=...
export OVH_SSH_HOST="mise-cache-prod.example-tailnet.ts.net"
export OVH_SSH_SOURCE_CIDR="$(tailscale ip -4)/32"
./deploy/ovh/deploy.sh
```

`OVH_SSH_HOST` selects a private DNS name, IP address, or OpenSSH host alias
without changing the public address used by DNS. It defaults to the Terraform
`server_ipv4` output. `OVH_SSH_SOURCE_CIDR` is required: there is no world-open
default. Mise checks the active SSH connection against this rule and
`OVH_SSH_PORT` before atomically applying the nftables policy, so an incorrect
CIDR or port fails before it can lock out the operator. The declared policy
permits TCP 80/443, UDP 443 for HTTP/3, SSH from the current deployment peer,
and SSH from the Tailscale CGNAT range only when traffic arrives on
`tailscale0`.

The deploy script reads the hostname, server address, R2 endpoint, and bucket
from Terraform outputs. It creates a mode-`0700` local bootstrap project,
writes only the explicitly required runtime values into mode-`0600` source
files, and runs `mise bootstrap remote`. Mise stages the project in another
mode-`0700` temporary directory on the host, converges the protected service
environment files, and removes the staging directory. A shell trap removes the
local project on success or failure. The caller's other environment variables
are never forwarded.

Automated deployments store `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, and
`MISE_CACHE_DATABASE_PASSWORD` in the repository's protected `production`
GitHub Environment. Environment protection keeps these values out of ordinary
pull-request jobs. The R2 credential remains limited by its Cloudflare bucket
policy even if a workflow is compromised. Use a local password manager to
populate the same variables for an emergency operator-run deployment; never
commit their plaintext values.

Ubuntu images use the `ubuntu` SSH user by default. Override `OVH_SSH_USER`,
`OVH_SSH_PORT`, `OVH_SSH_HOST`, or `OVH_SSH_IDENTITY_FILE` when necessary;
normal OpenSSH configuration and host-key policy still apply. Additional
arguments are passed to `mise bootstrap remote`, including repeatable
`--ssh-option` values for bastions and userspace-networking proxies. The
complete remote plan can be inspected without changing the host:

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
- other push workflows: read-only; and
- the exact `jdx/mise-cache` production deployment workflow: read/write for
  its isolated qualification namespace.

Override `MISE_CACHE_GITHUB_REPOSITORY` and `MISE_CACHE_GITHUB_OWNER_ID` when
deploying for another repository owner. Override
`MISE_CACHE_DEPLOY_GITHUB_REPOSITORY` and
`MISE_CACHE_DEPLOY_GITHUB_WORKFLOW_REF` together when deployment is managed by
another repository or workflow.

## Verify and operate

```sh
curl --fail "$(terraform -chdir=deploy/ovh/terraform output -raw cache_url)/v1/status"
ssh ubuntu@"$OVH_SSH_HOST" \
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
