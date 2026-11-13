// construct: jsx-component
export default function Widget({ title, children }) {
  return (
    <section>
      <h2>{title}</h2>
      {children}
    </section>
  );
}

// construct: jsx-arrow-component
export const Label = ({ text }) => <span className="label">{text}</span>;
