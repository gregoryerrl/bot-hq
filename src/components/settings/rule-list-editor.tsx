"use client";

import { useState, KeyboardEvent } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { X, Plus } from "lucide-react";

interface RuleListEditorProps {
  label: string;
  description?: string;
  items: string[];
  onChange: (items: string[]) => void;
  placeholder?: string;
}

export function RuleListEditor({
  label,
  description,
  items,
  onChange,
  placeholder = "Add item...",
}: RuleListEditorProps) {
  const [inputValue, setInputValue] = useState("");

  function addItem() {
    const trimmed = inputValue.trim();
    if (trimmed && !items.includes(trimmed)) {
      onChange([...items, trimmed]);
      setInputValue("");
    }
  }

  function removeItem(index: number) {
    onChange(items.filter((_, i) => i !== index));
  }

  function handleKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter") {
      e.preventDefault();
      addItem();
    }
  }

  return (
    <div className="space-y-3">
      <div>
        <label className="text-sm font-medium">{label}</label>
        {description && (
          <p className="text-xs text-muted-foreground mt-1">{description}</p>
        )}
      </div>

      <div className="flex gap-2">
        <Input
          value={inputValue}
          onChange={(e) => setInputValue(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          className="flex-1"
        />
        <Button type="button" size="icon" onClick={addItem} disabled={!inputValue.trim()}>
          <Plus className="h-4 w-4" />
        </Button>
      </div>

      {items.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {items.map((item, index) => (
            <Badge key={index} variant="secondary" className="pr-1">
              <code className="text-xs">{item}</code>
              <button
                type="button"
                onClick={() => removeItem(index)}
                className="ml-1 p-0.5 hover:bg-muted rounded"
              >
                <X className="h-3 w-3" />
              </button>
            </Badge>
          ))}
        </div>
      )}
    </div>
  );
}
