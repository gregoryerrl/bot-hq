"use client";

import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

interface TerminalViewProps {
  terminal: Terminal | null;
  fitAddon: FitAddon | null;
}

export function TerminalView({ terminal, fitAddon }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (terminal && containerRef.current) {
      // Clear container
      containerRef.current.innerHTML = "";
      // Open terminal in container
      terminal.open(containerRef.current);
      fitAddon?.fit();
    }
  }, [terminal, fitAddon]);

  // Handle window resize
  useEffect(() => {
    const handleResize = () => {
      fitAddon?.fit();
    };

    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [fitAddon]);

  return (
    <div
      ref={containerRef}
      className="h-full w-full bg-[#1a1b26]"
      style={{ minHeight: "400px" }}
    />
  );
}
