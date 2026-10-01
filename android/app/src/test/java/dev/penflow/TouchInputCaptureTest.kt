package dev.penflow

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TouchInputCaptureTest {

    private fun starts(x: Float, y: Float, topZone: Int = 60) =
        TouchInputCapture.startsOutsideForwardableArea(
            x, y, left = 0, top = 0, right = 2880, bottom = 1800, topZonePx = topZone,
        )

    @Test
    fun touchDownInTopGestureZoneIsExcluded() {
        assertTrue(starts(1440f, 0f))
        assertTrue(starts(1440f, 59f))
    }

    @Test
    fun touchDownBelowTopGestureZoneIsForwarded() {
        assertFalse(starts(1440f, 60f))
        assertFalse(starts(1440f, 900f))
    }

    @Test
    fun touchDownInLetterboxIsExcluded() {
        // Active rect narrower than the panel: bars at x < 200 and x > 2680.
        val inBar = TouchInputCapture.startsOutsideForwardableArea(
            100f, 900f, left = 200, top = 0, right = 2680, bottom = 1800, topZonePx = 60,
        )
        assertTrue(inBar)
    }

    @Test
    fun emptyRectOnlyAppliesTopZone() {
        val beforeHandshake = TouchInputCapture.startsOutsideForwardableArea(
            5000f, 900f, left = 0, top = 0, right = 0, bottom = 0, topZonePx = 60,
        )
        assertFalse(beforeHandshake)
    }
}
