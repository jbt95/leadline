CREATE TABLE public.users (id integer PRIMARY KEY, email text);
CREATE TABLE IF NOT EXISTS sessions (id integer PRIMARY KEY, expired boolean);
