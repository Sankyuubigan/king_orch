export function computeTargetIndex(centers: number[], currentCenter: number): number {
  if (centers.length === 0) return 0;
  let bestIdx = 0;
  let minDiff = Math.abs(centers[0] - currentCenter);
  for (let i = 1; i < centers.length; i++) {
    const diff = Math.abs(centers[i] - currentCenter);
    if (diff < minDiff) {
      minDiff = diff;
      bestIdx = i;
    }
  }
  return bestIdx;
}

export function computeShifts(
  count: number,
  startIdx: number,
  targetIdx: number,
  step: number,
): number[] {
  const shifts = new Array<number>(count).fill(0);
  if (startIdx === targetIdx) return shifts;
  if (startIdx < targetIdx) {
    for (let i = startIdx + 1; i <= targetIdx; i++) shifts[i] = -step;
  } else {
    for (let i = targetIdx; i < startIdx; i++) shifts[i] = step;
  }
  return shifts;
}
