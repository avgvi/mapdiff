// construct: side-effect-module

// construct: declare-global
declare global {
  interface Window {
    __shapes?: number;
  }
}

globalThis.console.debug("fixture loaded");

// construct: empty-export
export {};
