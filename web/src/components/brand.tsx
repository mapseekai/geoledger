/** GeoLedger mark; the upper glyph follows `currentColor` so it works on light and dark surfaces. */
export function Logo({ size = 28 }: { size?: number }) {
  return (
    <svg
      viewBox="0 0 64 72"
      width={size}
      height={Math.round((size * 72) / 64)}
      fill="none"
      aria-hidden="true"
      focusable="false"
    >
      <path
        fill="currentColor"
        d="M32 4 56 18 48 23 32 14 16 23V37L32 46 48 37V33H32V25H56V42L32 56 8 42V18Z"
      />
      <path fill="#fa520f" d="m8 47 24 14 24-14v8L32 69 8 55Z" />
    </svg>
  );
}
export function Brand({ caption }: { caption?: string }) {
  return (
    <span className="brand">
      <span className="brand-icon">
        <Logo />
      </span>
      <span className="brand-text">
        <span className="brand-name">
          GeoLedger<span className="brand-dot">_</span>
        </span>
        {caption && <small>{caption}</small>}
      </span>
    </span>
  );
}
