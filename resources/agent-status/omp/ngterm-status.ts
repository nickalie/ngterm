// ngterm-omp-status-extension

import { homedir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

export default function (omp: ExtensionAPI) {
  const wrapper = join(homedir(), ".config", "ngterm", "agent-status", "ngterm-agent-status.sh");
  const [program, prefix]: [string, string[]] =
    process.platform === "win32"
      ? [
          "powershell.exe",
          ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", wrapper.replace(/\.sh$/, ".ps1")],
        ]
      : [wrapper, []];

  async function report(args: string[]): Promise<void> {
    if (!process.env.NGTERM_SESSION_ID) return;
    try {
      await omp.exec(program, [...prefix, ...args], { timeout: 1_000 });
    } catch {
      // A status failure must not stop the agent.
    }
  }

  omp.on("agent_start", async () => {
    await report(["active", "--blink"]);
  });

  omp.on("agent_end", async (event) => {
    // Retries, queued continuations and background work have not finished the turn.
    await report(event.willContinue ? ["active", "--blink"] : ["completed", "--auto-reset"]);
  });

  omp.on("session_shutdown", async () => {
    await report(["idle"]);
  });
}
