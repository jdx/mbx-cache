CREATE TABLE IF NOT EXISTS action_manifests (
    namespace TEXT NOT NULL,
    algorithm TEXT NOT NULL CHECK (algorithm = 'blake3'),
    hash TEXT NOT NULL,
    size BIGINT NOT NULL CHECK (size >= 0),
    etag TEXT NOT NULL,
    manifest JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (namespace, algorithm, hash, size)
);
