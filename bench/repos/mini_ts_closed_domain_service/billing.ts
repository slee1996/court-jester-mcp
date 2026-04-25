import { BillingCycle } from "./billing-types.ts";

const DAYS: Partial<Record<BillingCycle, number>> = {
  [BillingCycle.Monthly]: 30,
};

export function cycleDays(cycle: BillingCycle): number {
  return DAYS[cycle]!.valueOf();
}
