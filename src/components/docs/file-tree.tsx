"use client";

import { useState } from "react";
import { ChevronRight, ChevronDown, Folder, FileText } from "lucide-react";
import { cn } from "@/lib/utils";

interface FileNode {
  path: string;
  type: "file" | "directory";
  children?: FileNode[];
}

interface FileTreeProps {
  files: FileNode[];
  selectedPath: string | null;
  onSelectFile: (path: string) => void;
}

export function FileTree({ files, selectedPath, onSelectFile }: FileTreeProps) {
  return (
    <div className="space-y-1">
      {files.map((node) => (
        <TreeNode
          key={node.path}
          node={node}
          level={0}
          selectedPath={selectedPath}
          onSelectFile={onSelectFile}
        />
      ))}
    </div>
  );
}

interface TreeNodeProps {
  node: FileNode;
  level: number;
  selectedPath: string | null;
  onSelectFile: (path: string) => void;
}

function TreeNode({ node, level, selectedPath, onSelectFile }: TreeNodeProps) {
  const [isExpanded, setIsExpanded] = useState(true);
  const isSelected = selectedPath === node.path;

  const handleClick = () => {
    if (node.type === "directory") {
      setIsExpanded(!isExpanded);
    } else {
      onSelectFile(node.path);
    }
  };

  const paddingLeft = level * 16;

  return (
    <div>
      <button
        onClick={handleClick}
        className={cn(
          "w-full flex items-center gap-2 px-2 py-1.5 text-sm rounded hover:bg-muted transition-colors text-left",
          isSelected && "bg-muted font-medium"
        )}
        style={{ paddingLeft: `${paddingLeft + 8}px` }}
      >
        {node.type === "directory" ? (
          <>
            {isExpanded ? (
              <ChevronDown className="h-4 w-4 flex-shrink-0" />
            ) : (
              <ChevronRight className="h-4 w-4 flex-shrink-0" />
            )}
            <Folder className="h-4 w-4 flex-shrink-0 text-muted-foreground" />
          </>
        ) : (
          <>
            <div className="w-4" />
            <FileText className="h-4 w-4 flex-shrink-0 text-muted-foreground" />
          </>
        )}
        <span className="truncate">{getDisplayName(node.path)}</span>
      </button>

      {node.type === "directory" && isExpanded && node.children && (
        <div>
          {node.children.map((child) => (
            <TreeNode
              key={child.path}
              node={child}
              level={level + 1}
              selectedPath={selectedPath}
              onSelectFile={onSelectFile}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function getDisplayName(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1];
}
