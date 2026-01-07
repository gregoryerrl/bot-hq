import { db, settings } from "@/lib/db";
import { eq } from "drizzle-orm";
import path from "path";
import os from "os";

const SCOPE_PATH_KEY = "scope_path";
const DEFAULT_SCOPE_PATH = path.join(os.homedir(), "Projects");

/**
 * Get the configured scope directory path with fallback logic:
 * 1. Check database settings
 * 2. Fall back to SCOPE_PATH env var
 * 3. Fall back to ~/Projects
 */
export async function getScopePath(): Promise<string> {
  try {
    // Try to get from database
    const result = await db
      .select()
      .from(settings)
      .where(eq(settings.key, SCOPE_PATH_KEY))
      .limit(1);

    if (result.length > 0) {
      return result[0].value;
    }
  } catch (error) {
    console.error("Failed to read scope_path from database:", error);
  }

  // Fall back to environment variable
  if (process.env.SCOPE_PATH) {
    return process.env.SCOPE_PATH;
  }

  // Fall back to default
  return DEFAULT_SCOPE_PATH;
}

/**
 * Set the scope directory path in the database
 */
export async function setScopePath(scopePath: string): Promise<void> {
  await db
    .insert(settings)
    .values({
      key: SCOPE_PATH_KEY,
      value: scopePath,
      updatedAt: new Date(),
    })
    .onConflictDoUpdate({
      target: settings.key,
      set: {
        value: scopePath,
        updatedAt: new Date(),
      },
    });
}

/**
 * Get a setting by key
 */
export async function getSetting(key: string): Promise<string | null> {
  try {
    const result = await db
      .select()
      .from(settings)
      .where(eq(settings.key, key))
      .limit(1);

    return result.length > 0 ? result[0].value : null;
  } catch (error) {
    console.error(`Failed to read setting ${key} from database:`, error);
    return null;
  }
}

/**
 * Set a setting in the database
 */
export async function setSetting(key: string, value: string): Promise<void> {
  await db
    .insert(settings)
    .values({
      key,
      value,
      updatedAt: new Date(),
    })
    .onConflictDoUpdate({
      target: settings.key,
      set: {
        value,
        updatedAt: new Date(),
      },
    });
}
