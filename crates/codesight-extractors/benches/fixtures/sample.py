"""Bench fixture: a small Python module."""

from collections import defaultdict
from typing import Protocol


class Greeter(Protocol):
    def greet(self, name: str) -> str: ...


class EnglishGreeter:
    def greet(self, name: str) -> str:
        return f"Hello, {name}!"


class SpanishGreeter:
    def greet(self, name: str) -> str:
        return f"¡Hola, {name}!"


def pick_greeter(language: str) -> Greeter:
    if language == "es":
        return SpanishGreeter()
    return EnglishGreeter()


class Counter:
    def __init__(self) -> None:
        self._inner: dict[str, int] = defaultdict(int)

    def bump(self, key: str) -> int:
        self._inner[key] += 1
        return self._inner[key]

    def total(self) -> int:
        return sum(self._inner.values())


def double(x: int) -> int:
    return x * 2


class Wrapper:
    def __init__(self, inner: object) -> None:
        self.inner = inner

    def cloned(self) -> object:
        return self.inner
