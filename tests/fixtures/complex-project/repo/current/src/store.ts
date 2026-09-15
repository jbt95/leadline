export function saveOrder(ids: number[], db: any = globalThis): void {
  for (const id of ids) {
    db.query("UPDATE orders SET processed = true WHERE id = " + id);
  }
}
