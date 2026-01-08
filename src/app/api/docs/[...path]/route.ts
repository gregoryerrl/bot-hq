import { NextRequest, NextResponse } from "next/server";
import { readDocument, getDocumentPath } from "@/lib/agent-docs";
import fs from "fs";

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ path: string[] }> }
) {
  try {
    const { path } = await params;
    const relativePath = path.join("/");

    // Check if this is a download request
    const url = new URL(request.url);
    const isDownload = url.pathname.endsWith("/download");

    if (isDownload) {
      // Handle download
      const fullPath = await getDocumentPath(relativePath.replace(/\/download$/, ""));
      const filename = fullPath.split("/").pop() || "document.md";

      // Read file as stream
      const fileBuffer = fs.readFileSync(fullPath);

      return new NextResponse(fileBuffer, {
        headers: {
          "Content-Type": "text/markdown",
          "Content-Disposition": `attachment; filename="${filename}"`,
        },
      });
    } else {
      // Handle read
      const { content, filename } = await readDocument(relativePath);

      return NextResponse.json({
        content,
        filename,
        path: relativePath,
      });
    }
  } catch (error) {
    console.error("Failed to access document:", error);
    const message = error instanceof Error ? error.message : "Failed to access document";

    if (message.includes("Access denied") || message.includes("Invalid path")) {
      return NextResponse.json({ error: message }, { status: 403 });
    }

    if (message.includes("ENOENT") || message.includes("not found")) {
      return NextResponse.json({ error: "Document not found" }, { status: 404 });
    }

    return NextResponse.json({ error: message }, { status: 500 });
  }
}
