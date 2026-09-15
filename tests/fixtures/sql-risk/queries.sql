-- safe baseline select
SELECT id, email FROM public.users WHERE email = 'LITERAL_SENTINEL';
-- risky: update without where
UPDATE users SET active = false;
-- risky: leading wildcard
SELECT * FROM users WHERE email LIKE '%example.com';
-- risky: nonsargable predicate
SELECT * FROM users WHERE lower(email) = 'a@b.c';
-- risky: large offset
SELECT * FROM users ORDER BY id LIMIT 10 OFFSET 1001;
-- safe offset boundary
SELECT * FROM users ORDER BY id LIMIT 10 OFFSET 1000;
-- safe: update and delete with where
UPDATE users SET active = false WHERE id = 1;
DELETE FROM sessions WHERE expired = true;
-- risky: unknown table
SELECT * FROM ghost WHERE id = 1;
