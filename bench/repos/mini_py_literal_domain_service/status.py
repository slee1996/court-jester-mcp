from typing import Literal


def status_label(status: Literal["draft", "published"]) -> str:
    if status == "draft":
        return "Draft"

    raise ValueError(f"unsupported status: {status}")
