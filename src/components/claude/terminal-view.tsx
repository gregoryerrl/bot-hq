"use client";

import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

interface TerminalViewProps {
  terminal: Terminal | null;
  fitAddon: FitAddon | null;
  isVisible: boolean;
}

export function TerminalView({ terminal, fitAddon, isVisible }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const isOpenedRef = useRef(false);

  // Open terminal only once AND only when visible
  useEffect(() => {
    if (terminal && containerRef.current && isVisible && !isOpenedRef.current) {
      terminal.open(containerRef.current);
      isOpenedRef.current = true;
      // Small delay to ensure DOM is ready
      requestAnimationFrame(() => {
        fitAddon?.fit();
      });
    }
  }, [terminal, fitAddon, isVisible]);

  // Reset opened ref when terminal changes (new session)
  useEffect(() => {
    isOpenedRef.current = false;
  }, [terminal]);

  // Refit when becoming visible (after already opened)
  useEffect(() => {
    if (isVisible && terminal && isOpenedRef.current) {
      // Small delay to ensure container is rendered
      requestAnimationFrame(() => {
        fitAddon?.fit();
      });
    }
  }, [isVisible, terminal, fitAddon]);

  // Handle window resize
  useEffect(() => {
    const handleResize = () => {
      if (isVisible) {
        fitAddon?.fit();
      }
    };

    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [fitAddon, isVisible]);

  return (
    <div
      ref={containerRef}
      className="h-full w-full bg-[#1a1b26]"
      style={{
        minHeight: "400px",
        display: isVisible ? "block" : "none"
      }}
    />
  );
}
