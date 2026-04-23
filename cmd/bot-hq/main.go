package main

import (
	"fmt"
	"os"
)

func main() {
	if len(os.Args) > 1 {
		switch os.Args[1] {
		case "mcp":
			fmt.Println("MCP server mode — not yet implemented")
			os.Exit(0)
		case "status":
			fmt.Println("Status — not yet implemented")
			os.Exit(0)
		case "version":
			fmt.Println("bot-hq v0.1.0")
			os.Exit(0)
		}
	}
	fmt.Println("Bot-HQ Hub — not yet implemented")
}
