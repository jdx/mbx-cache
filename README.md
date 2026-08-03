# mise-cache

`mise-cache` is the official self-hostable remote cache service for mise tasks. It implements version 1 of the mise content-addressed cache protocol with immutable blobs, atomic action-result commits, namespace isolation, and streaming transfers.

## Features

- BLAKE3 and SHA-256 content-addressed storage
- Filesystem storage for a single-node installation
- S3-compatible storage for production, including MinIO
- In-memory metadata for development or PostgreSQL for durable, horizontally scalable deployments
- Static and OIDC bearer authorization with per-namespace read/write grants
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
