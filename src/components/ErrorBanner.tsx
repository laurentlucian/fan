export default function ErrorBanner({
  text,
  onDismiss,
}: {
  text: string;
  onDismiss: () => void;
}) {
  return (
    <div className="banner" role="alert">
      <span className="banner-text">{text}</span>
      <button className="banner-x" title="Dismiss" onClick={onDismiss}>
        ✕
      </button>
    </div>
  );
}
