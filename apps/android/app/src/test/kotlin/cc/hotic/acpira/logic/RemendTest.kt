package cc.hotic.acpira.logic

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Test
import java.io.File
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * The Kotlin remend port against the installed remend's own output: the iOS app's conformance cases
 * (apps/ios RemendCases.generated.swift, written by scripts/export/ios-remend.ts with JSON string literals), read in
 * place rather than exported a second time. Mirrors apps/ios RemendTests.swift
 */
class RemendTest {
  private val source = File("../../ios/Packages/AcpiraKit/Tests/AcpiraKitTests/RemendCases.generated.swift").readText()
  private val literal = Regex(""""(?:[^"\\]|\\.)*"""")

  private fun decode(s: String) = Json.parseToJsonElement(s).jsonPrimitive.content

  /** Each streamed document and remend's output for every prefix: `.append(tail)` or `.full(text)` */
  private fun documents(): List<Pair<String, List<Pair<Boolean, String>>>> {
    val body = source.substringAfter("static let documents").substringBefore("static let edges")
    val out = ArrayList<Pair<String, MutableList<Pair<Boolean, String>>>>()
    val entry = Regex("""^    \(("(?:[^"\\]|\\.)*"), \[$""")
    val case = Regex("""\.(append|full)\(("(?:[^"\\]|\\.)*")\)""")
    for (line in body.lines()) {
      val doc = entry.find(line)
      if (doc != null) { out += decode(doc.groupValues[1]) to ArrayList(); continue }
      for (m in case.findAll(line)) out.last().second += (m.groupValues[1] == "append") to decode(m.groupValues[2])
    }
    return out
  }

  @Test fun streamedPrefixesMatchRemend() {
    val docs = documents()
    assertTrue(docs.sumOf { it.second.size } > 1000)
    for ((text, cases) in docs) {
      cases.forEachIndexed { i, (append, value) ->
        // A prefix that splits a surrogate pair never streams
        if (i > 0 && text[i - 1].isHighSurrogate()) return@forEachIndexed
        val prefix = text.substring(0, i)
        assertEquals(if (append) prefix + value else value, Remend.repair(prefix), "prefix $i: $prefix")
      }
    }
  }

  @Test fun edgesMatchRemend() {
    val body = source.substringAfter("static let edges")
    val edges = body.lines().mapNotNull { line ->
      val parts = literal.findAll(line).map { decode(it.value) }.toList()
      if (line.trimStart().startsWith("(\"") && parts.size == 2) parts[0] to parts[1] else null
    }
    // The edges exercise every repair, not just pass-through
    assertTrue(edges.count { it.first != it.second } > 30)
    for ((input, output) in edges) assertEquals(output, Remend.repair(input), input)
  }
}
