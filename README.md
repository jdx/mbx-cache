# mise-cache

`mise-cache` is the official self-hostable remote build-cache service for mise. It implements version 1 of the mise action-cache protocol with immutable blobs, atomic action-result commits, namespace isolation, typed action schemas, and streaming transfers.

## Features

- BLAKE3 and SHA-256 content-addressed storage
- Filesystem storage for a single-node installation
- S3-compatible storage for production, including MinIO
- In-memory metadata for development or PostgreSQL for durable, horizontally scalable deployments
- Static and OIDC bearer authorization with per-namespace read/write grants
- Recursive output-tree validation before an action result becomes visible
- Typed action and client-metadata validation with kind negotiation
- Batch missing-blob queries, streaming blob packs, and Prometheus metrics
- Docker Compose, Helm, and Terraform-managed OVH deployments

## Quick start

Install the service from crates.io:

```sh
cargo install mise-cache
```

The development stack starts the service, PostgreSQL, and MinIO:

```sh
docker compose up --build
```

It listens on `http://localhost:8080`. The included token is `development-token` and permits the `default` namespace. Change it before exposing the service.

For a local filesystem-backed instance:

```sh
cargo run -- \
  --allow-anonymous \
  --data-dir ./data \
  --listen 127.0.0.1:8080
```

Anonymous access is intended only for a trusted local network. Production installations should terminate TLS at an ingress or proxy and configure tokens.

## Configuration

Every option has a matching environment variable and CLI flag. Run `mise-cache --help` for the complete list.

| Environment variable | Default | Purpose |
|---|---:|---|
| `MISE_CACHE_LISTEN` | `0.0.0.0:8080` | Listen address |
| `MISE_CACHE_STORAGE` | `filesystem` | `filesystem` or `s3` |
| `MISE_CACHE_DATA_DIR` | `/var/lib/mise-cache` | Filesystem blob root |
| `MISE_CACHE_DATABASE_URL` | `memory://` | PostgreSQL URL or development memory store |
| `MISE_CACHE_S3_BUCKET` | — | Required for S3 storage |
| `MISE_CACHE_S3_PREFIX` | `v1` | Object-key prefix |
| `MISE_CACHE_S3_ENDPOINT` | AWS default | S3-compatible endpoint |
| `MISE_CACHE_S3_REGION` | `us-east-1` | S3 region |
| `MISE_CACHE_S3_PATH_STYLE` | `false` | Enable for MinIO and similar services |
| `MISE_CACHE_TOKENS_JSON` | — | Token grants, described below |
| `MISE_CACHE_OIDC_PROVIDERS_JSON` | — | Trusted OIDC providers and claim grants, described below |
| `MISE_CACHE_ALLOW_ANONYMOUS` | `false` | Allow access without configured grants |
| `MISE_CACHE_MAX_BLOB_BYTES` | `5368709120` | Maximum upload size |

AWS credentials use the standard AWS SDK credential chain, including environment variables, workload identity, ECS, and EC2 roles.

### Authorization

`MISE_CACHE_TOKENS_JSON` is an array of grants. Namespace patterns may be an exact name, `*`, or a prefix ending in `/*`.

```json
[
  {
    "token": "replace-with-a-secret",
    "read": ["acme/*", "public"],
    "write": ["acme/project-a"]
  }
]
```

Rotate tokens by deploying the old and new grants together, moving clients to the new token, and then removing the old grant. Do not put the JSON directly in a Helm values file; inject it through a Kubernetes Secret.

OIDC lets CI systems use short-lived identity tokens instead of stored cache secrets. Configure trusted issuers, acceptable audiences, and one or more claim-based grants with `MISE_CACHE_OIDC_PROVIDERS_JSON`:

```json
[
  {
    "issuer": "https://token.actions.githubusercontent.com",
    "audiences": ["https://cache.example.com"],
    "rules": [
      {
        "claims": {
          "repository": "jdx/mise",
          "repository_owner_id": "216188"
        },
        "read": ["jdx/mise"],
        "write": ["jdx/mise"]
      }
    ]
  }
]
```

The server discovers the issuer's JWKS endpoint, verifies the signature, issuer, audience, expiry, not-before time, and subject, then applies the first matching authorization rule. Rules are alternatives; every claim within a rule must match exactly. A configured claim may be an array of accepted scalar values, and a token claim may itself be an array. Namespace grants use the same exact, `*`, and `prefix/*` forms as static tokens.

Authorization is deny-by-default: every provider needs at least one audience and rule, and every rule must constrain at least one claim. Pin stable identity claims such as GitHub's numeric `repository_owner_id` alongside the repository name; add `ref`, `environment`, or other claims when only a narrower workflow identity should be able to write. Symmetric JWT algorithms are never accepted.

Optional provider settings are:

| Field | Default | Purpose |
|---|---:|---|
| `discovery_uri` | `<issuer>/.well-known/openid-configuration` | Override OIDC discovery |
| `jwks_uri` | discovered | Use an explicit JWKS endpoint and skip discovery |
| `algorithms` | supported asymmetric algorithms | Restrict accepted JWT signing algorithms |
| `jwks_refresh_seconds` | `300` | Refresh interval; an unknown key ID also requests a refresh |
| `clock_skew_seconds` | `60` | Leeway for expiry and not-before validation |

The service makes three bounded attempts to fetch provider metadata and keys at startup. It refreshes stale keys during token validation, and an unknown key ID requests a JWKS refresh. Refresh attempts have a 30-second cooldown to prevent attacker-controlled key IDs from amplifying outbound requests. If a periodic refresh temporarily fails, an already-cached key remains usable; an unknown key never does. Use HTTPS for discovery and JWKS endpoints outside a trusted private network.

### GitHub Actions OIDC

Until mise can acquire the job identity token itself, a workflow can request it from GitHub and pass it through the existing cache-token environment variable:

```yaml
permissions:
  contents: read
  id-token: write

steps:
  - uses: actions/checkout@v4
  - name: acquire cache identity
    id: cache-identity
    env:
      OIDC_AUDIENCE: https://cache.example.com
    run: |
      response="$(curl --fail --silent --show-error \
        -H "Authorization: bearer $ACTIONS_ID_TOKEN_REQUEST_TOKEN" \
        "$ACTIONS_ID_TOKEN_REQUEST_URL&audience=$OIDC_AUDIENCE")"
      token="$(jq -r .value <<<"$response")"
      echo "::add-mask::$token"
      echo "token=$token" >> "$GITHUB_OUTPUT"
  - run: mise run test
    env:
      MISE_TASK_CACHE_REMOTE_TOKEN: ${{ steps.cache-identity.outputs.token }}
```

Treat the output as a secret even though it is short-lived. Set the audience in the workflow to exactly one of the provider's configured audiences.

## Deployment

- `docker-compose.yml` runs a local development stack with PostgreSQL and MinIO.
- `charts/mise-cache` runs a horizontally scalable Kubernetes deployment.
- [`deploy/ovh`](deploy/ovh/README.md) provisions a low-cost US production
  instance with Terraform and converges its host with `mise bootstrap remote`.
  Cache blobs use Cloudflare R2.

## API

All cache requests send `Mise-Cache-Namespace` and, unless anonymous access is enabled, `Authorization: Bearer …`.

- `GET /v1/status`
- `GET /v1/capabilities`
- `GET|PUT /v1/blobs/{algorithm}/{hash}/{size}`
- `POST /v1/blobs:missing`
- `POST /v1/blobs:pack`
- `GET|PUT /v1/action-results/{algorithm}/{hash}/{size}`
- `GET|PUT /v1/action-manifests/{algorithm}/{hash}/{size}`
- `GET /metrics`

Blobs and action results are immutable. Repeating an identical write is idempotent; attempting to replace an existing action result returns `409 Conflict`. The server verifies uploaded content and every blob reachable from result metadata and output trees before publishing an action result. Content digests inside an action descriptor identify local inputs for key construction; they are not CAS references, and clients do not upload source inputs.

Task action manifests are mutable discovery indexes for fresh workers. Their stable key is the BLAKE3 digest of the canonical task-manifest selector. Writes use optimistic concurrency: create with `If-None-Match: *`, or update the ETag returned by `GET` with `If-Match`. A stale update returns `412 Precondition Failed`, so clients must read, merge, and retry without dropping actions learned by another worker.

Servers advertising `features.blob_packs` accept the same digest-list JSON as `blobs:missing` at `POST /v1/blobs:pack`. The response media type is `application/vnd.mise.cache-blob-pack.v1`. It begins with the eight-byte `MISEPK01` magic and then streams visible blobs in request order. Each blob is framed by a one-byte algorithm (`1` for BLAKE3, `2` for SHA-256), its raw 32-byte hash, an unsigned big-endian 64-bit size, and exactly that many content bytes. Missing or unauthorized blobs are omitted, duplicate requests are emitted once, and clients must verify every digest before admitting content to local CAS. The aggregate declared size is bounded by `MISE_CACHE_MAX_BLOB_BYTES` and advertised as `limits.max_pack_bytes`.

`GET /v1/capabilities` advertises the action kinds and exact schema versions accepted by the server. Action-result keys use BLAKE3. Version 1 accepts task and rustc action and metadata schema version 1. Rustc results require an output directory tree plus metadata referencing raw stdout and stderr blobs so clients can replay compiler diagnostics byte-for-byte.

## Operations

Run multiple stateless replicas against the same PostgreSQL database and S3 bucket. Readiness and liveness probes use `/v1/status`. Scrape `/metrics` with Prometheus.

Do not expire S3 objects independently of PostgreSQL metadata. The metadata store currently assumes a registered blob remains present, so independent object expiration can leave action results pointing at missing blobs. Until coordinated garbage collection is implemented, monitor storage growth and reset the blob and metadata stores together when reclaiming space.

Back up PostgreSQL and enable S3 versioning or replication as required. The service never exposes a deletion endpoint, so retention and disaster recovery remain administrative concerns.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

The protocol specification and mise client live in [jdx/mise](https://github.com/jdx/mise).

## License

MIT
