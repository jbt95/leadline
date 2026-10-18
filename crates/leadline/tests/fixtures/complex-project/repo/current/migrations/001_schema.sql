CREATE TABLE public.users (
  id bigint PRIMARY KEY,
  email text NOT NULL,
  active boolean NOT NULL DEFAULT true
);

CREATE TABLE public.orders (
  id bigint PRIMARY KEY,
  processed boolean NOT NULL DEFAULT false
);
