"use client";
import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { Confirm } from "./common";

/** Protect local edits on dialog dismissal, internal navigation and browser exit. */
export function useUnsavedChanges(dirty: boolean) {
  const router = useRouter();
  const active = useRef(dirty);
  active.current = dirty;
  const [pending, setPending] = useState<(() => void) | null>(null);
  const guard = (leave: () => void) => {
    if (dirty) setPending(() => leave);
    else leave();
  };
  useEffect(() => {
    if (!dirty) return;
    const unload = (event: BeforeUnloadEvent) => {
      if (!active.current) return;
      event.preventDefault();
      event.returnValue = "";
    };
    const navigate = (event: MouseEvent) => {
      if (
        !active.current ||
        event.defaultPrevented ||
        event.button !== 0 ||
        event.metaKey ||
        event.ctrlKey ||
        event.shiftKey ||
        event.altKey
      )
        return;
      const link =
        event.target instanceof Element
          ? event.target.closest("a[href]")
          : null;
      if (
        !(link instanceof HTMLAnchorElement) ||
        link.download ||
        (link.target && link.target !== "_self")
      )
        return;
      const url = new URL(link.href);
      if (
        url.origin !== location.origin ||
        url.pathname + url.search === location.pathname + location.search
      )
        return;
      event.preventDefault();
      event.stopPropagation();
      setPending(() => () => router.push(url.pathname + url.search + url.hash));
    };
    window.addEventListener("beforeunload", unload);
    document.addEventListener("click", navigate, true);
    return () => {
      window.removeEventListener("beforeunload", unload);
      document.removeEventListener("click", navigate, true);
    };
  }, [dirty, router]);
  return {
    guard,
    markSaved: () => {
      active.current = false;
    },
    confirmation: pending && (
      <Confirm
        title="放弃未保存的修改？"
        description="修改尚未保存，离开后将丢失。"
        action="放弃修改"
        busy={false}
        error=""
        close={() => setPending(null)}
        confirm={() => {
          active.current = false;
          setPending(null);
          pending();
        }}
      />
    ),
  };
}
