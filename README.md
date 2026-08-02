# mise-cache

`mise-cache` is the official self-hostable remote cache service for mise tasks. It implements version 1 of the mise content-addressed cache protocol with immutable blobs, atomic action-result commits, namespace isolation, and streaming transfers.

## Features

- BLAKE3 and SHA-256 content-addressed storage
- Filesystem storage for a single-node installation
- S3-compatible storage for production, including MinIO
- In-memory metadata for development or PostgreSQL for durable, horizontally scalable deployments
- Bearer-token authorization with per-namespace read/write grants
- Recursive output-tree validation before an action result becomes visible
- Batch missing-blob queries and Prometheus metrics
- Docker Compose and Helm deployments

## Quick start

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

## API

All cache requests send `Mise-Cache-Namespace` and, unless anonymous access is enabled, `Authorization: Bearer …`.

- `GET /v1/status`
- `GET /v1/capabilities`
- `GET|PUT /v1/blobs/{algorithm}/{hash}/{size}`
- `POST /v1/blobs:missing`
- `GET|PUT /v1/action-results/{algorithm}/{hash}/{size}`
- `GET /metrics`

Blobs and action results are immutable. Repeating an identical write is idempotent; attempting to replace an existing action result returns `409 Conflict`. The server verifies uploaded content and all referenced blobs before publishing an action result.

## Operations

Run multiple stateless replicas against the same PostgreSQL database and S3 bucket. Readiness and liveness probes use `/v1/status`. Scrape `/metrics` with Prometheus. S3 lifecycle policies control retention; PostgreSQL records are deliberately small and may be retained or expired by an operator-supplied job according to organizational policy.

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

