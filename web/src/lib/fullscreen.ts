/** Best-effort exit before opening another view; a denied exit must not block it. */
export async function leaveFullscreen(): Promise<void> {
  try {
    if (document.fullscreenElement) await document.exitFullscreen();
  } catch {
    // The document may already be leaving fullscreen or no longer be active.
  }
}

/** State remains driven by fullscreenchange; only explicit user actions report failure. */
export async function toggleFullscreen(
  element: Pick<HTMLElement, "requestFullscreen"> | null,
  onError: () => void,
): Promise<void> {
  try {
    if (document.fullscreenElement) await document.exitFullscreen();
    else await element?.requestFullscreen?.();
  } catch {
    onError();
  }
}
