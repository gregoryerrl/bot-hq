import { NextRequest, NextResponse } from "next/server";
import { exec } from "child_process";
import { promisify } from "util";

const execAsync = promisify(exec);

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { startPath } = body;

    // Use osascript (AppleScript) on macOS to show folder picker
    const script = `
      tell application "System Events"
        activate
        set folderPath to choose folder ${startPath ? `default location "${startPath}"` : ""} with prompt "Select workspace folder"
        return POSIX path of folderPath
      end tell
    `;

    const { stdout } = await execAsync(`osascript -e '${script.replace(/'/g, "'\\''")}'`);
    const path = stdout.trim();

    return NextResponse.json({ path });
  } catch (error) {
    console.error("Failed to pick folder:", error);
    // User likely cancelled the picker
    return NextResponse.json({ path: null });
  }
}
