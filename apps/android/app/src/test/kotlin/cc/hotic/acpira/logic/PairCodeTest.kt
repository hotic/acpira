package cc.hotic.acpira.logic

import com.google.zxing.BinaryBitmap
import com.google.zxing.RGBLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.decoder.Decoder
import com.google.zxing.qrcode.detector.Detector
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

/** The pairing code's rows and rings (apps/ios PairCode.swift, pairGeometry.ts qrShape) */
class PairCodeTest {
  private val link = "acpira://pair/v1/Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6"

  @Test fun knockOutFollowsTheCodeSize() {
    assertEquals(11, PairCodeShape.holeOf(57))
    assertEquals(7, PairCodeShape.holeOf(21))
    // Odd on an odd code, so it sits on whole modules
    for (n in listOf(21, 25, 33, 41, 49, 57, 65)) assertEquals(1, PairCodeShape.holeOf(n) % 2)
  }

  @Test fun everyDarkModuleOutsideTheEyesAndHoleSitsInOneRing() {
    val rows = assertNotNull(PairCodeRows.make(link))
    val n = rows.size
    assertTrue(rows.all { it.length == n } && (n - 17) % 4 == 0)
    val shape = PairCodeShape(rows)
    val h0 = (n - shape.hole) / 2
    val expected = rows.indices.sumOf { y ->
      rows[y].indices.count { x ->
        val eye = (x < 7 && y < 7) || (x >= n - 7 && y < 7) || (x < 7 && y >= n - 7)
        val hole = x in h0 until h0 + shape.hole && y in h0 until h0 + shape.hole
        rows[y][x] == '1' && !eye && !hole
      }
    }
    assertEquals(expected, shape.dots.sumOf { it.size })
    assertEquals(3, shape.eyes.sumOf { it.size })
    assertEquals(PairCodeShape.BANDS, shape.dots.size)
  }

  @Test fun aScannedGridReadsBackAsTheSameRows() {
    // The code drawn as pixels, then detected the way the camera reads it: the sampled grid is the code's own modules
    val rows = assertNotNull(PairCodeRows.make(link))
    val scale = 4
    val quiet = 4
    val side = (rows.size + 2 * quiet) * scale
    val pixels = IntArray(side * side) { i ->
      val x = i % side / scale - quiet
      val y = i / side / scale - quiet
      if (y in rows.indices && x in rows.indices && rows[y][x] == '1') 0xFF000000.toInt() else 0xFFFFFFFF.toInt()
    }
    val found = Detector(BinaryBitmap(HybridBinarizer(RGBLuminanceSource(side, side, pixels))).blackMatrix).detect()
    assertEquals(rows, PairCodeRows.of(found.bits.clone()))
    assertEquals(link, Decoder().decode(found.bits).text)
  }
}
