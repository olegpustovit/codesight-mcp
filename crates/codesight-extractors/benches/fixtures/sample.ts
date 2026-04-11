// Bench fixture: a small TypeScript module.

export interface Greeter {
    greet(name: string): string;
}

export class EnglishGreeter implements Greeter {
    greet(name: string): string {
        return `Hello, ${name}!`;
    }
}

export class SpanishGreeter implements Greeter {
    greet(name: string): string {
        return `¡Hola, ${name}!`;
    }
}

export function pickGreeter(language: string): Greeter {
    if (language === "es") {
        return new SpanishGreeter();
    }
    return new EnglishGreeter();
}

export type Bumps = Map<string, number>;

export class Counter {
    private inner: Bumps = new Map();

    bump(key: string): number {
        const next = (this.inner.get(key) ?? 0) + 1;
        this.inner.set(key, next);
        return next;
    }

    total(): number {
        let sum = 0;
        for (const value of this.inner.values()) {
            sum += value;
        }
        return sum;
    }
}

export namespace nested {
    export function double(x: number): number {
        return x * 2;
    }

    export class Wrapper<T> {
        constructor(public inner: T) {}
    }
}
