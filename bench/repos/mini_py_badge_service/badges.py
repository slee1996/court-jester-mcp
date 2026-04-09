def referral_points(referrals: int | None) -> int | None:
    if referrals is None:
        return None
    return referrals * 10


def badge_for_referrals(referrals: int | None) -> str:
    points = referral_points(referrals)
    if points >= 50:
        return "gold"
    if points >= 20:
        return "silver"
    return "starter"
