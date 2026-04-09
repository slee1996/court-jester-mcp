import { defaultFlags } from "./defaults.ts";

export type Config = {
  flags?: {
    betaCheckout?: boolean | null;
  } | null;
} | null;

export function betaCheckoutEnabled(config: Config): boolean {
  return config?.flags?.betaCheckout || defaultFlags().betaCheckout;
}
