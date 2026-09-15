UPDATE users SET active = false;
SELECT * FROM users WHERE email LIKE '%example.com';
SELECT * FROM users WHERE lower(email) = 'a@example.com';
SELECT * FROM users ORDER BY id LIMIT 20 OFFSET 1001;
SELECT * FROM ghost WHERE id = 1;
