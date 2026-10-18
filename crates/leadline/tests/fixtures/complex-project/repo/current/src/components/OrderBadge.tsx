import { priceOrder } from "../service";

export const OrderBadge = ({ items }: { items: number[] }) => {
  const total = priceOrder(items, "US");
  return <strong>{total > 100 ? "priority" : "standard"}</strong>;
};
