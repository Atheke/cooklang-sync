CREATE TABLE file_records (
  id INTEGER PRIMARY KEY NOT NULL,
  jid INTEGER,
  deleted INTEGER NOT NULL DEFAULT(FALSE),
  path VARCHAR NOT NULL, -- relative to storage dir
  format CHARACTER(1) NOT NULL,
  modified_at DATETIME NOT NULL,
  size BIGINT NOT NULL
)
