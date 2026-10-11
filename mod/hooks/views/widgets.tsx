import type { BoxProps, ButtonProps, ElementConstructor, RasterProps, TextProps } from 'claude-code'

import type { Grid } from '../chart'
import { encode, runs } from '../chart'
import type { Palette } from '../palette'
import type { Line, Segment } from '../state'

export type Ui = {
  Box: ElementConstructor<BoxProps>
  Text: ElementConstructor<TextProps>
  Button: ElementConstructor<ButtonProps>
  Raster?: ElementConstructor<RasterProps>
}

export function Piece({ ui, segment }: { ui: Ui; segment: Segment }) {
  const { Text } = ui
  return (
    <Text color={segment.color} dimColor={segment.dim} bold={segment.bold}>
      {segment.text}
    </Text>
  )
}

export const RASTER_MAX_COLUMNS = 512
export const RASTER_MAX_ROWS = 256

export function Chart({ ui, chartKey, grid, width, palette }: { ui: Ui; chartKey: string; grid: Grid; width: number; palette: Palette | null }) {
  const { Box, Text, Raster } = ui
  const fits = grid.columns <= RASTER_MAX_COLUMNS && grid.rows <= RASTER_MAX_ROWS
  if (Raster !== undefined && palette !== null && fits) return <Raster key={chartKey} columns={grid.columns} rows={grid.rows} cells={encode(grid, palette)} />
  return (
    <Box key={chartKey} flexDirection="column" width={width}>
      {runs(grid).map((row, index) => (
        <Text key={`${chartKey}-${index}`} wrap="truncate-end">
          {row.map(run => (
            <Text color={run.fg ?? undefined} backgroundColor={run.bg ?? undefined}>
              {run.text}
            </Text>
          ))}
        </Text>
      ))}
    </Box>
  )
}

export function Row({ ui, line, width, palette, onToggle }: { ui: Ui; line: Line; width: number; palette: Palette | null; onToggle: (key: string) => void }) {
  const { Box, Text, Button } = ui
  if (line.chart !== undefined) return <Chart ui={ui} chartKey={line.key} grid={line.chart} width={width} palette={palette} />
  const pieces = (
    <Text wrap={line.wrap === true ? 'wrap' : 'truncate-end'}>
      {line.segments.map(segment => (
        <Piece ui={ui} segment={segment} />
      ))}
    </Text>
  )
  const toggle = line.toggle
  const text =
    toggle === undefined ? (
      pieces
    ) : (
      <Button key={toggle} plain onPress={() => onToggle(toggle)}>
        {pieces}
      </Button>
    )
  const why = line.why
  if (why === undefined) {
    return (
      <Box key={line.key} width={width} paddingLeft={line.indent ?? 0}>
        {text}
      </Box>
    )
  }
  return (
    <Box key={line.key} width={width} justifyContent="space-between">
      {text}
      <Button key={`why-${why}`} plain dimColor label="?" onPress={() => onToggle(why)} />
    </Box>
  )
}
