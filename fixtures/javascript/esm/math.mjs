export function add(a, b) {
  return a + b;
}

export const sub = (a, b) => a - b;

function mul(a, b) {
  return a * b;
}

// construct: esm-export-list
export { mul as multiply };
