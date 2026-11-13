// construct: const
export const UNITS = ["m", "ft"] as const;

// construct: type-alias-indexed
export type Unit = (typeof UNITS)[number];

// construct: let
let conversions = 0;

// construct: function
export function toMeters(value: number, unit: Unit): number {
  conversions += 1;
  return unit === "ft" ? value * 0.3048 : value;
}

// construct: module-private-function
function round(value: number): number {
  return Math.round(value * 100) / 100;
}

// construct: function-overload
export function format(value: number): string;
export function format(value: number, unit: Unit): string;
export function format(value: number, unit?: Unit): string {
  return unit ? `${round(value)} ${unit}` : String(round(value));
}

// construct: generic-function
export function first<T>(items: readonly T[]): T | undefined {
  return items[0];
}

// construct: arrow-const
export const double = (value: number): number => value * 2;

// construct: function-expression-const
export const triple = function (value: number): number {
  return value * 3;
};

// construct: async-function
export async function load(): Promise<Unit[]> {
  return [...UNITS];
}

// construct: generator-function
export function* steps(limit: number): Generator<number> {
  for (let step = 0; step < limit; step += 1) {
    yield step;
  }
}

// construct: object-literal-method
export const converter = {
  feet(value: number): number {
    return value / 0.3048;
  },
  // construct: object-literal-arrow
  meters: (value: number): number => value,
};

// construct: export-list
export { conversions as conversionCount };
