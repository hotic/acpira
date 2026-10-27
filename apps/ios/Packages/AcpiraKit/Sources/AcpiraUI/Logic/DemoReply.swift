import AcpiraModels
import Foundation

// The reply a prompt gets while no engine is connected: a short thought, one read in the project, and a paragraph
// saying where the reply comes from and echoing the prompt. Streamed by AppModel through Replay's steps, so a sent
// prompt shows the live turn (Orb, Working label, paced text) without a CLI behind it.

enum DemoReply {
  static func turn(for prompt: String, cwd: String, at date: Date) -> AgentTurn {
    let started = date.timeIntervalSince1970 * 1000
    let quoted = prompt.split(whereSeparator: \.isNewline).map { "> \($0)" }.joined(separator: "\n")
    return AgentTurn(
      blocks: [
        .thought(ThoughtBlock(text: t("mobile.demo.thought"), startedAt: started, durationSec: 2)),
        .toolCall(ToolCallBlock(id: "demo-read-\(Int(started))", kind: .read, verb: t("verb.read"), target: "README.md",
                                targetMono: true, status: .completed)),
        .text(TextBlock(markdown: t("mobile.demo.reply", ["project": cwd.split(separator: "/").last.map(String.init) ?? cwd])
                        + (quoted.isEmpty ? "" : "\n\n" + quoted))),
      ],
      startedAt: started, endedAt: started + 3000, stop: .end_turn)
  }
}
