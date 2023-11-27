
CREATE TABLE IF NOT EXISTS assistant(
  id VARCHAR PRIMARY KEY,
  name VARCHAR NOT NULL,
  description TEXT
);

CREATE TABLE IF NOT EXISTS message(
  id VARCHAR PRIMARY KEY,
  created_at INTEGER NOT NULL,
  text_content TEXT NOT NULL,
  thread_id VARCHAR NOT NULL,
  role_id INTEGER NOT NULL ,
  assistant_id VARCHAR,
  FOREIGN KEY(thread_id) REFERENCES thread(id),
  FOREIGN KEY(assistant_id) REFERENCES assistant(id),
);


CREATE TABLE IF NOT EXISTS thread( id VARCHAR PRIMARY KEY);


