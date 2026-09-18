export default function Meter({
  level,
  wide = false,
}: {
  level: number;
  wide?: boolean;
}) {
  return (
    <div className={wide ? "meter meter-wide" : "meter"}>
      <div
        className="meter-fill"
        data-hot={level > 0.96 || undefined}
        style={{ transform: `scaleX(${level.toFixed(3)})` }}
      />
    </div>
  );
}
