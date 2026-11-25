# AudioTestReference decisions

## Slice 5 — Low-pass parameter precision and readback

NIH-plug stores continuous controls as binary32. Promote interior values to binary64 and map the binary32 endpoint values to the contract bounds so decimal endpoints remain selectable without relaxing domain validation. Use the shared coefficient calculation in the processor and editor. Display the last DSP-applied cutoff/Q separately from the editable host settings so pending block-boundary edits are not presented as effective. Derive Hz from the active host rate; do not assume a rate when inactive.
