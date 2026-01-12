import { describe, it, expect } from "vitest";
import { parseGitHubRemote } from "@/lib/github/types";

describe("parseGitHubRemote", () => {
  it("parses owner/repo format", () => {
    const result = parseGitHubRemote("owner/repo");
    expect(result).toEqual({
      owner: "owner",
      name: "repo",
      fullName: "owner/repo",
    });
  });

  it("parses HTTPS URL", () => {
    const result = parseGitHubRemote("https://github.com/owner/repo");
    expect(result).toEqual({
      owner: "owner",
      name: "repo",
      fullName: "owner/repo",
    });
  });

  it("parses HTTPS URL with .git suffix", () => {
    const result = parseGitHubRemote("https://github.com/owner/repo.git");
    expect(result).toEqual({
      owner: "owner",
      name: "repo",
      fullName: "owner/repo",
    });
  });

  it("parses SSH URL", () => {
    const result = parseGitHubRemote("git@github.com:owner/repo");
    expect(result).toEqual({
      owner: "owner",
      name: "repo",
      fullName: "owner/repo",
    });
  });

  it("parses SSH URL with .git suffix", () => {
    const result = parseGitHubRemote("git@github.com:owner/repo.git");
    expect(result).toEqual({
      owner: "owner",
      name: "repo",
      fullName: "owner/repo",
    });
  });

  it("returns null for invalid remote", () => {
    expect(parseGitHubRemote("invalid")).toBeNull();
    expect(parseGitHubRemote("")).toBeNull();
  });

  it("handles repos with hyphens and underscores", () => {
    const result = parseGitHubRemote("my-org/my_repo-name");
    expect(result).toEqual({
      owner: "my-org",
      name: "my_repo-name",
      fullName: "my-org/my_repo-name",
    });
  });
});
