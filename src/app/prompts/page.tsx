"use client";

import { useState, useEffect } from "react";
import { Header } from "@/components/layout/header";
import { PromptList } from "@/components/prompts/prompt-list";
import { PromptEditor, EmptyPromptEditor } from "@/components/prompts/prompt-editor";

interface PromptData {
  id: number;
  slug: string;
  name: string;
  description: string | null;
  content: string;
  variables: string | null;
  isParametric: boolean;
  updatedAt: string;
}

export default function PromptsPage() {
  const [prompts, setPrompts] = useState<PromptData[]>([]);
  const [selectedSlug, setSelectedSlug] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetchPrompts();
  }, []);

  async function fetchPrompts() {
    try {
      const res = await fetch("/api/prompts");
      const data = await res.json();
      setPrompts(data);

      // Auto-select first prompt if none selected
      if (!selectedSlug && data.length > 0) {
        setSelectedSlug(data[0].slug);
      }
    } catch (error) {
      console.error("Failed to fetch prompts:", error);
    } finally {
      setLoading(false);
    }
  }

  const selectedPrompt = prompts.find((p) => p.slug === selectedSlug) || null;

  if (loading) {
    return (
      <div className="flex flex-col h-full">
        <Header title="Prompts" description="Manage agent prompts" />
        <div className="flex-1 p-6 flex items-center justify-center">
          <div className="text-muted-foreground">Loading prompts...</div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <Header title="Prompts" description="Manage agent prompts" />
      <div className="flex-1 flex overflow-hidden">
        {/* Prompt list sidebar */}
        <div className="w-64 border-r overflow-auto p-3">
          {prompts.length === 0 ? (
            <div className="text-sm text-muted-foreground p-2">
              No prompts found. They will be seeded on first load.
            </div>
          ) : (
            <PromptList
              prompts={prompts}
              selectedSlug={selectedSlug}
              onSelect={setSelectedSlug}
            />
          )}
        </div>

        {/* Prompt editor */}
        <div className="flex-1 overflow-hidden">
          {selectedPrompt ? (
            <PromptEditor
              key={selectedPrompt.slug}
              prompt={selectedPrompt}
              onSaved={fetchPrompts}
            />
          ) : (
            <EmptyPromptEditor />
          )}
        </div>
      </div>
    </div>
  );
}
