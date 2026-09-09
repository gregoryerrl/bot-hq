import { describe, it, expect } from "vitest";
import { uriListToPaths } from "./filePaste";

describe("uriListToPaths", () => {
  it("decodes file URIs, one per line, skipping comments and non-files", () => {
    expect(
      uriListToPaths(
        "# comment\nfile:///tmp/a%20b.md\r\nhttps://x.com/c.md\nfile:///Users/x/c.png\n",
      ),
    ).toEqual(["/tmp/a b.md", "/Users/x/c.png"]);
  });

  it("returns nothing for plain text", () => {
    expect(uriListToPaths("just some words")).toEqual([]);
  });
});

