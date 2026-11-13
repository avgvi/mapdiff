import type { Shape } from "./models";

// construct: tsx-props-interface
interface BadgeProps {
  shape: Shape;
  onSelect?: (shape: Shape) => void;
}

// construct: tsx-component
export function Badge({ shape, onSelect }: BadgeProps) {
  return (
    <button type="button" onClick={() => onSelect?.(shape)}>
      {shape.label ?? shape.kind}
    </button>
  );
}

// construct: tsx-arrow-component
export const BadgeList = ({ shapes }: { shapes: Shape[] }) => (
  <ul>
    {shapes.map((shape) => (
      <li key={shape.kind}>
        <Badge shape={shape} />
      </li>
    ))}
  </ul>
);

// construct: export-default-identifier
export default Badge;
