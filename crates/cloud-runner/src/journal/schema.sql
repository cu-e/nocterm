BEGIN IMMEDIATE;
CREATE TABLE account_catalog (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  revision INTEGER NOT NULL CHECK(revision>0),
  body BLOB NOT NULL
);
CREATE TABLE chats (
  chat BLOB PRIMARY KEY CHECK(length(chat)=32),
  binding BLOB NOT NULL CHECK(length(binding)=32),
  scope BLOB NOT NULL CHECK(length(scope)=32),
  chat_id BLOB NOT NULL CHECK(length(chat_id)=16),
  runner BLOB NOT NULL CHECK(length(runner)=16),
  key_epoch INTEGER NOT NULL CHECK(key_epoch>0),
  writer INTEGER NOT NULL CHECK(writer>0),
  revision INTEGER NOT NULL CHECK(revision>=0),
  records INTEGER NOT NULL CHECK(records>=0),
  events INTEGER NOT NULL CHECK(events>=0),
  control INTEGER NOT NULL CHECK(control>=0),
  retained INTEGER NOT NULL CHECK(retained>=0),
  deleted INTEGER NOT NULL CHECK(deleted IN(0,1)),
  charge INTEGER NOT NULL CHECK(charge>=0),
  body BLOB NOT NULL,
  header_digest BLOB NOT NULL CHECK(length(header_digest)=32)
) WITHOUT ROWID;
CREATE TABLE current_heads (
  chat BLOB PRIMARY KEY REFERENCES chats(chat),
  revision INTEGER NOT NULL CHECK(revision>0),
  writer INTEGER CHECK(writer IS NULL OR writer>0),
  digest BLOB NOT NULL CHECK(length(digest)=32),
  body BLOB NOT NULL
) WITHOUT ROWID;
CREATE TABLE entries (
  chat BLOB NOT NULL REFERENCES chats(chat),
  sequence INTEGER NOT NULL CHECK(sequence>0),
  revision INTEGER NOT NULL CHECK(revision>0),
  writer INTEGER NOT NULL CHECK(writer>0),
  charge INTEGER NOT NULL CHECK(charge>0),
  body BLOB NOT NULL,
  PRIMARY KEY(chat,sequence)
) WITHOUT ROWID;
CREATE TABLE commands (
  chat BLOB NOT NULL REFERENCES chats(chat),
  id BLOB NOT NULL CHECK(length(id)=16),
  revision INTEGER NOT NULL CHECK(revision>0),
  sequence INTEGER NOT NULL CHECK(sequence>0),
  writer INTEGER NOT NULL CHECK(writer>0),
  body BLOB NOT NULL,
  PRIMARY KEY(chat,id)
) WITHOUT ROWID;
CREATE TABLE checkpoints (
  chat BLOB PRIMARY KEY REFERENCES chats(chat),
  revision INTEGER NOT NULL CHECK(revision>0),
  sequence INTEGER NOT NULL CHECK(sequence>0),
  writer INTEGER NOT NULL CHECK(writer>0),
  body BLOB NOT NULL
) WITHOUT ROWID;
CREATE TABLE outbox (
  chat BLOB NOT NULL REFERENCES chats(chat),
  sequence INTEGER NOT NULL CHECK(sequence>0),
  id BLOB NOT NULL CHECK(length(id)=16),
  delivery BLOB NOT NULL CHECK(length(delivery)=16),
  record INTEGER NOT NULL CHECK(record>0),
  revision INTEGER NOT NULL CHECK(revision>0),
  writer INTEGER NOT NULL CHECK(writer>0),
  body BLOB NOT NULL,
  acknowledged INTEGER NOT NULL DEFAULT 0 CHECK(acknowledged IN(0,1)),
  PRIMARY KEY(chat,sequence),
  UNIQUE(chat,id),
  UNIQUE(chat,delivery)
) WITHOUT ROWID;
CREATE TABLE outbox_deliveries (
  chat BLOB NOT NULL REFERENCES chats(chat),
  sequence INTEGER NOT NULL CHECK(sequence>0),
  body BLOB NOT NULL,
  PRIMARY KEY(chat,sequence)
) WITHOUT ROWID;
CREATE TABLE snapshots (
  chat BLOB PRIMARY KEY REFERENCES chats(chat),
  revision INTEGER NOT NULL CHECK(revision>0),
  sequence INTEGER NOT NULL CHECK(sequence>0),
  writer INTEGER NOT NULL CHECK(writer>0),
  body BLOB NOT NULL
) WITHOUT ROWID;
CREATE TABLE run_heads (
  chat BLOB NOT NULL REFERENCES chats(chat),
  run BLOB NOT NULL CHECK(length(run)=32),
  epoch INTEGER NOT NULL CHECK(epoch>0),
  revision INTEGER NOT NULL CHECK(revision>0),
  sequence INTEGER NOT NULL CHECK(sequence>0),
  stream_revision INTEGER NOT NULL CHECK(stream_revision>0),
  writer INTEGER NOT NULL CHECK(writer>0),
  body BLOB NOT NULL,
  PRIMARY KEY(chat,run)
) WITHOUT ROWID;
CREATE TABLE effects (
  chat BLOB NOT NULL REFERENCES chats(chat),
  scope BLOB NOT NULL CHECK(length(scope)=32),
  chat_id BLOB NOT NULL CHECK(length(chat_id)=16),
  runner BLOB NOT NULL CHECK(length(runner)=16),
  key_epoch INTEGER NOT NULL CHECK(key_epoch>0),
  intent BLOB NOT NULL CHECK(length(intent)=16),
  state INTEGER NOT NULL CHECK(state BETWEEN 0 AND 4),
  sequence INTEGER NOT NULL CHECK(sequence>0),
  revision INTEGER NOT NULL CHECK(revision>0),
  writer INTEGER NOT NULL CHECK(writer>0),
  body BLOB NOT NULL,
  PRIMARY KEY(chat,intent)
) WITHOUT ROWID;
CREATE INDEX pending_effects ON effects(scope,state);
CREATE TABLE snapshot_transfers (
  chat BLOB PRIMARY KEY REFERENCES chats(chat),
  revision INTEGER NOT NULL CHECK(revision>0),
  sequence INTEGER NOT NULL CHECK(sequence>0),
  writer INTEGER NOT NULL CHECK(writer>0),
  body BLOB NOT NULL
) WITHOUT ROWID;
CREATE TABLE snapshot_payloads (
  chat BLOB NOT NULL REFERENCES chats(chat),
  kind INTEGER NOT NULL CHECK(kind BETWEEN 1 AND 3),
  revision INTEGER NOT NULL CHECK(revision>0),
  sequence INTEGER NOT NULL CHECK(sequence>0),
  writer INTEGER NOT NULL CHECK(writer>0),
  body BLOB NOT NULL,
  PRIMARY KEY(chat,kind)
) WITHOUT ROWID;
PRAGMA user_version=5;
COMMIT;
