export type Order = {
  billing?: {
    country?: string | null;
  } | null;
} | null;

export function billingCountry(order: Order): string {
  return order!.billing!.country!.trim().toUpperCase();
}
