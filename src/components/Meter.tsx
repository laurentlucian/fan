export default function Meter({
  level,
  variant = "tile",
}: {
  level: number;
  variant?: "tile" | "hero";
}) {
  return (
    <div className={variant === "hero" ? "meter meter-hero" : "meter"}>
      <div
        className="meter-fill"
        data-hot={level > 0.96 || undefined}
        style={{ transform: `scaleX(${level.toFixed(3)})` }}
      />
    </div>
  );
}
