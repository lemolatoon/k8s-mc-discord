# k8s-mc-discord

Discord bot that bridges a Minecraft server and exposes slash commands to interact with a separate API.

## Commands

- `/list`: Calls `GET {MC_API_BASE}/list` and returns the response text.
- `/kill`: Calls `POST {MC_API_BASE}/kill` to delete/kill the Minecraft pod, then reports the API response.

## Config

Set the following environment variables:

- `DISCORD_TOKEN`: Discord bot token
- `DISCORD_CHANNEL`: Channel ID to mirror messages
- `MC_API_BASE`: Base URL of the Minecraft API (e.g., `http://host:8080`)

The bot also connects to `{MC_API_BASE}` WebSocket at `/chats` for Minecraft ↔ Discord chat relay.
