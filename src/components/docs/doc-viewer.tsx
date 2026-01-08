"use client";

import { Button } from "@/components/ui/button";
import { Download, FileQuestion } from "lucide-react";
import ReactMarkdown from "react-markdown";

interface DocViewerProps {
  content: string;
  filename: string;
  path: string;
  onDownload: () => void;
}

export function DocViewer({ content, filename, path, onDownload }: DocViewerProps) {
  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between p-4 border-b">
        <div className="flex flex-col">
          <h2 className="text-lg font-semibold">{filename}</h2>
          <p className="text-sm text-muted-foreground">{path}</p>
        </div>
        <Button onClick={onDownload} variant="outline" size="sm">
          <Download className="h-4 w-4 mr-2" />
          Download
        </Button>
      </div>
      <div className="flex-1 overflow-auto p-6">
        <div className="prose prose-slate dark:prose-invert max-w-none">
          <ReactMarkdown>{content}</ReactMarkdown>
        </div>
      </div>
    </div>
  );
}

export function EmptyDocViewer() {
  return (
    <div className="flex flex-col items-center justify-center h-full p-8 text-center">
      <FileQuestion className="h-16 w-16 text-muted-foreground mb-4" />
      <h2 className="text-xl font-semibold mb-2">No Document Selected</h2>
      <p className="text-muted-foreground max-w-md">
        Select a document from the file tree on the left to view its contents.
      </p>
    </div>
  );
}
