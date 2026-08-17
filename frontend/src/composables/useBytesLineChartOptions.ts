// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ChartOptions, TooltipItem } from 'chart.js'
import { formatBytes } from '../utils/format'
import { useChartTheme } from './useChartTheme'

/**
 * Shared chart.js option factory for the byte-valued line charts on the
 * dashboard (storage/backup-size trends). Handles theme-reactive colors and
 * the formatBytes tooltip/axis labels; callers only choose the legend.
 */
export function useBytesLineChartOptions(): {
  bytesLineOptions: (showLegend: boolean) => ChartOptions<'line'>
} {
  const { textMuted, border } = useChartTheme()

  function bytesTooltipLabel(context: TooltipItem<'line'>): string {
    return `${context.dataset.label ?? ''}: ${formatBytes(context.parsed.y ?? 0)}`
  }

  function bytesLineOptions(showLegend: boolean): ChartOptions<'line'> {
    return {
      responsive: true,
      maintainAspectRatio: false,
      interaction: { intersect: false, mode: 'index' },
      plugins: {
        legend: showLegend
          ? { display: true, labels: { color: textMuted.value, boxWidth: 12, font: { size: 10 } } }
          : { display: false },
        tooltip: { callbacks: { label: bytesTooltipLabel } },
      },
      scales: {
        x: { grid: { display: false }, ticks: { color: textMuted.value, font: { size: 10 } } },
        y: {
          grace: '10%',
          grid: { color: border.value },
          ticks: {
            color: textMuted.value,
            font: { size: 10 },
            callback: (value: string | number): string => formatBytes(Number(value)),
          },
        },
      },
    }
  }

  return { bytesLineOptions }
}
