-- Your SQL goes here
CREATE TABLE samples (
  id BLOB PRIMARY KEY NOT NULL,
  name TEXT NOT NULL,
  description TEXT,
  status TEXT NOT NULL,
  tags TEXT,
  image_uri TEXT,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP
)
