import { betaCheckoutEnabled } from "../flags.ts";

if (betaCheckoutEnabled(null) !== true) {
  throw new Error("expected null config to fall back to true");
}

if (betaCheckoutEnabled({ flags: { betaCheckout: true } }) !== true) {
  throw new Error("expected explicit true to stay true");
}
