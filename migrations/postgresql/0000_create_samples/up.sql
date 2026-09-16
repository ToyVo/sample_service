-- Your SQL goes here
-- NOTE: SQLite uses BLOB for `id`; Postgres uses BYTEA (both map to Diesel `Binary`).
-- Keep the two `migrations/*/0000_create_samples/up.sql` files in sync (column names/order).
CREATE TABLE samples (
  id BYTEA PRIMARY KEY NOT NULL,
  name TEXT NOT NULL,
  description TEXT,
  status TEXT NOT NULL,
  tags TEXT,
  image_uri TEXT,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP
)
