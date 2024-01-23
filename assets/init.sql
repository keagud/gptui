CREATE TABLE thread(
  id VARCHAR PRIMARY KEY,
  model VARCHAR
);

CREATE TABLE message(
  thread_id VARCHAR,
  role INTEGER,
  content VARCHAR,
  timestamp FLOAT,
  tokens INTEGER,
  FOREIGN KEY (thread_id) REFERENCES thread (id)
);

CREATE TABLE title(
  id VARCHAR PRIMARY KEY,
  content TEXT
);


CREATE TABLE summary(
  thread_id VARCHAR,
  start_index INTEGER,
  end_index INTEGER,
  content TEXT,
  FOREIGN KEY (thread_id) REFERENCES thread(id)
);
