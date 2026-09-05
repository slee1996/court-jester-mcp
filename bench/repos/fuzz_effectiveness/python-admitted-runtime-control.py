from typing import Literal


def first_character(value: Literal['', 'a']) -> str:
    return value[0] if value else ''
