// Minimal ambient types for the `node:test` / `node:assert/strict` surface the suites use, so
// `tsc --noEmit` type-checks the tests without installing `@types/node` (zero dependencies).
declare module 'node:test' {
  export function test(name: string, fn: () => void | Promise<void>): Promise<void>;
}

declare module 'node:assert/strict' {
  interface Assert {
    (value: unknown, message?: string): asserts value;
    equal<T>(actual: unknown, expected: T, message?: string): asserts actual is T;
    notEqual(actual: unknown, expected: unknown, message?: string): void;
    deepEqual<T>(actual: unknown, expected: T, message?: string): asserts actual is T;
    ok(value: unknown, message?: string): asserts value;
    throws(fn: () => unknown, expected?: ((err: unknown) => boolean) | (abstract new (...args: never[]) => Error), message?: string): void;
  }
  const assert: Assert;
  export default assert;
}
