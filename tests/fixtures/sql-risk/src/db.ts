import { Pool } from 'pg';
const pool = new Pool();
export async function getUser(id: number) {
  return pool.query('SELECT * FROM users WHERE id = $1', [id]);
}
export async function findUsers(name: string) {
  return pool.query('SELECT * FROM users WHERE name LIKE ' + name);
}
export async function allUsers(names: string[]) {
  for (const name of names) {
    await pool.query(`SELECT * FROM users WHERE name = ${name}`);
  }
}
