CREATE TABLE IF NOT EXISTS action_results (
    namespace TEXT NOT NULL,
    algorithm TEXT NOT NULL CHECK (algorithm IN ('blake3', 'sha256')),
    hash TEXT NOT NULL,
    size BIGINT NOT NULL CHECK (size >= 0),
    result JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (namespace, algorithm, hash, size)
);

CREATE TABLE IF NOT EXISTS namespace_blobs (
    namespace TEXT NOT NULL,
    algorithm TEXT NOT NULL CHECK (algorithm IN ('blake3', 'sha256')),
    hash TEXT NOT NULL,
    size BIGINT NOT NULL CHECK (size >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_accessed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (namespace, algorithm, hash, size)
);
