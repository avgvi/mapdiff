// Hand-made fixture. Every TypeScript construct the extractor must map appears
// in this folder, tagged with a `construct:` marker on the line above it. The
// list of required markers is `fixtures/constructs/typescript.txt`.

// construct: import-type
import type { Unit } from "./units";
// construct: import-named
import { toMeters, UNITS } from "./units";

// construct: interface
export interface Shape {
  // construct: interface-property
  readonly kind: string;
  // construct: interface-method
  area(): number;
}

// construct: interface-merge
export interface Shape {
  label?: string;
}

// construct: type-alias
export type ShapeMap = Record<string, Shape>;

// construct: type-alias-union
export type Polygon =
  | { kind: "triangle"; base: number; height: number }
  | { kind: "rect"; width: number; height: number };

// construct: enum
export enum Color {
  // construct: enum-member
  Red = "red",
  Blue = "blue",
}

// construct: const-enum
export const enum Corner {
  TopLeft,
  TopRight,
}

// construct: abstract-class, class-implements
export abstract class BaseShape implements Shape {
  // construct: property
  label?: string;

  // construct: static-member
  static count = 0;

  // construct: access-protected
  protected unit: Unit = UNITS[0];

  // construct: access-hash-private
  #id = 0;

  // construct: abstract-property
  abstract readonly kind: string;

  // construct: abstract-method
  abstract area(): number;

  // construct: method
  describe(): string {
    this.bump();
    return `${this.kind} #${this.#id} with area ${this.area().toFixed(2)}`;
  }

  // construct: access-private
  private bump(): void {
    this.#id += 1;
  }
}

// construct: class, class-extends
export class Circle extends BaseShape {
  readonly kind = "circle";

  // construct: constructor, constructor-parameter-property
  constructor(
    public radius: number,
    unit: Unit = "m",
  ) {
    super();
    this.unit = unit;
    BaseShape.count += 1;
  }

  // construct: method-override
  override area(): number {
    return Math.PI * toMeters(this.radius, this.unit) ** 2;
  }

  // construct: getter
  get diameter(): number {
    return this.radius * 2;
  }

  // construct: setter
  set diameter(value: number) {
    this.radius = value / 2;
  }

  // construct: static-method
  static unit(): Circle {
    return new Circle(1);
  }
}

// construct: namespace
export namespace Geometry {
  // construct: namespace-interface
  export interface Point {
    x: number;
    y: number;
  }

  // construct: namespace-const
  export const ORIGIN: Point = { x: 0, y: 0 };

  // construct: namespace-function
  export function distance(a: Point, b: Point): number {
    return Math.hypot(a.x - b.x, a.y - b.y);
  }
}

// construct: declare
declare const __DEV__: boolean;
