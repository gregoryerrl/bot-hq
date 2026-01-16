"use client";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ChevronRight, Search } from "lucide-react";
import type { SelectionMenu as SelectionMenuType } from "@/lib/terminal-parser";

interface SelectionMenuProps {
  menu: SelectionMenuType;
  onSelect: (index: number) => void;
  onCancel: () => void;
}

export function SelectionMenu({ menu, onSelect, onCancel }: SelectionMenuProps) {
  return (
    <Card className="w-full max-w-2xl">
      <CardHeader className="pb-3">
        <CardTitle className="text-lg flex items-center gap-2">
          {menu.title}
        </CardTitle>
        {menu.hasSearch && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Search className="h-4 w-4" />
            <span>Use terminal view for search</span>
          </div>
        )}
      </CardHeader>
      <CardContent className="space-y-2">
        {menu.items.map((item, index) => (
          <Button
            key={index}
            variant={item.isSelected ? "default" : "outline"}
            className="w-full justify-start h-auto py-3 px-4"
            onClick={() => onSelect(index)}
          >
            <div className="flex items-center gap-3 w-full">
              <ChevronRight className="h-4 w-4 flex-shrink-0" />
              <div className="flex flex-col items-start text-left overflow-hidden">
                <span className="font-medium truncate max-w-full">
                  {item.label}
                </span>
                {item.description && (
                  <span className="text-xs text-muted-foreground truncate max-w-full">
                    {item.description}
                  </span>
                )}
              </div>
            </div>
          </Button>
        ))}
        <div className="pt-2 flex justify-between items-center">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            Cancel (Esc)
          </Button>
          {menu.instructions && (
            <span className="text-xs text-muted-foreground">
              {menu.instructions}
            </span>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
