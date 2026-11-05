-- Keep the two `migrations/*/0001_create_auth/up.sql` files in sync (column names/order).
-- `username` is stored lowercased; `password_hash` is a PHC-format argon2id string.
CREATE TABLE users (
  id BLOB PRIMARY KEY NOT NULL,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL
);

-- `passkey` is the JSON-serialized `webauthn_rs::prelude::Passkey`.
CREATE TABLE passkeys (
  id BLOB PRIMARY KEY NOT NULL,
  user_id BLOB NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  credential_id BLOB NOT NULL UNIQUE,
  passkey TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL,
  last_used_at TIMESTAMP
);

CREATE INDEX passkeys_user_id ON passkeys (user_id);
