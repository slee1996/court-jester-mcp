from typing import Literal


def count_billable_actions(actions: list[Literal["create", "delete"]]) -> int:
    count = 0
    for action in actions:
        if action == "create":
            count += 1
            continue
        raise ValueError(f"unsupported action: {action}")
    return count
