# Homebrew Cask for bot-hq.
#
# Distributed via an OWN TAP (the official homebrew-cask repo has notability
# requirements a brand-new app won't meet). Publish by creating the tap repo
# `gregoryerrl/homebrew-bot-hq`, copying this file to `Casks/bot-hq.rb`, then:
#
#   brew install --cask gregoryerrl/bot-hq/bot-hq
#
# On each release: bump `version`, refresh `sha256` (shasum -a 256 of the
# universal .dmg from the GitHub release), and copy this file to the tap.

cask "bot-hq" do
  # sha256 = shasum -a 256 of the published release's universal .dmg, verified
  # against the live download URL at publish time (2026-08-25).
  version "1.0.0"
  sha256 "c9ce0c0179f5952548cf49b67937c1723facff42f2db85a367431220dfffa18a"

  url "https://github.com/gregoryerrl/bot-hq/releases/download/v#{version}/bot-hq_#{version}_universal.dmg",
      verified: "github.com/gregoryerrl/bot-hq/"
  name "bot-hq"
  desc "Desktop GUI for AI-assisted coding sessions"
  homepage "https://github.com/gregoryerrl/bot-hq"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :catalina" # matches tauri.conf.json minimumSystemVersion 10.15

  app "bot-hq.app"

  caveats <<~EOS
    bot-hq runs each agent as a `claude-code` subprocess, so the claude-code
    CLI must be installed and authenticated:
      https://docs.claude.com/en/docs/claude-code

    This build is not yet signed or notarized by Apple. On first launch
    Gatekeeper may block it — right-click bot-hq.app and choose Open, or run:
      xattr -dr com.apple.quarantine "#{appdir}/bot-hq.app"

    Your data lives in ~/.bot-hq/ and is NOT removed on uninstall.
  EOS
end
