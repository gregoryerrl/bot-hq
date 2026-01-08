"use client";

import { useState, useEffect } from "react";
import { Header } from "@/components/layout/header";
import { FileTree } from "@/components/docs/file-tree";
import { DocViewer, EmptyDocViewer } from "@/components/docs/doc-viewer";

interface FileNode {
  path: string;
  type: "file" | "directory";
  children?: FileNode[];
}

interface DocumentContent {
  content: string;
  filename: string;
  path: string;
}

export default function DocsPage() {
  const [files, setFiles] = useState<FileNode[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [document, setDocument] = useState<DocumentContent | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadingDoc, setLoadingDoc] = useState(false);

  useEffect(() => {
    fetchFiles();
  }, []);

  async function fetchFiles() {
    try {
      const res = await fetch("/api/docs");
      const data = await res.json();
      setFiles(data.files || []);

      // Auto-select README.md if it exists
      const readme = data.files?.find((f: FileNode) => f.path === "README.md");
      if (readme) {
        loadDocument(readme.path);
      }
    } catch (error) {
      console.error("Failed to fetch documents:", error);
    } finally {
      setLoading(false);
    }
  }

  async function loadDocument(path: string) {
    setSelectedPath(path);
    setLoadingDoc(true);
    try {
      const res = await fetch(`/api/docs/${path}`);
      if (!res.ok) throw new Error("Failed to load document");

      const data = await res.json();
      setDocument(data);
    } catch (error) {
      console.error("Failed to load document:", error);
      setDocument(null);
    } finally {
      setLoadingDoc(false);
    }
  }

  async function handleDownload() {
    if (!selectedPath) return;

    try {
      const res = await fetch(`/api/docs/${selectedPath}/download`);
      if (!res.ok) throw new Error("Failed to download document");

      const blob = await res.blob();
      const url = window.URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = document?.filename || "document.md";
      document.body.appendChild(a);
      a.click();
      window.URL.revokeObjectURL(url);
      document.body.removeChild(a);
    } catch (error) {
      console.error("Failed to download document:", error);
    }
  }

  if (loading) {
    return (
      <div className="flex flex-col h-full">
        <Header title="Docs" description="Documentation written by agents" />
        <div className="flex-1 p-6 flex items-center justify-center">
          <div className="text-muted-foreground">Loading documents...</div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <Header title="Docs" description="Documentation written by agents" />
      <div className="flex-1 flex overflow-hidden">
        {/* File tree sidebar */}
        <div className="w-64 border-r overflow-auto p-4">
          {files.length === 0 ? (
            <div className="text-sm text-muted-foreground">
              No documents yet. Agents will create documentation here.
            </div>
          ) : (
            <FileTree
              files={files}
              selectedPath={selectedPath}
              onSelectFile={loadDocument}
            />
          )}
        </div>

        {/* Document viewer */}
        <div className="flex-1 overflow-hidden">
          {loadingDoc ? (
            <div className="flex items-center justify-center h-full">
              <div className="text-muted-foreground">Loading document...</div>
            </div>
          ) : document ? (
            <DocViewer
              content={document.content}
              filename={document.filename}
              path={document.path}
              onDownload={handleDownload}
            />
          ) : (
            <EmptyDocViewer />
          )}
        </div>
      </div>
    </div>
  );
}
