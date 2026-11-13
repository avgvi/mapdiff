// Minimal JSX types, so the fixture type-checks without installing React.

// construct: declaration-file, declare-namespace
declare namespace JSX {
  interface Element {
    readonly __element?: true;
  }

  interface IntrinsicElements {
    [name: string]: Record<string, unknown>;
  }
}
