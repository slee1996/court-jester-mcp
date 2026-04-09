from defaults import default_flags


def beta_checkout_enabled(config: dict | None) -> bool:
    return config.get("flags", {}).get("beta_checkout") or default_flags()["beta_checkout"]
