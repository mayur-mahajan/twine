/* Memory map of the linking fixtures (an STM32F4-like part). The images are only linked and
   their symbol tables inspected; they are never flashed. `Reset` is the only root: everything
   else is linked because it is reachable from it (`--gc-sections`). */
ENTRY(Reset);
EXTERN(Reset);

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 2M
  RAM   : ORIGIN = 0x20000000, LENGTH = 512K
}

SECTIONS
{
  .text : { *(.text .text.*); } > FLASH
  .rodata : { *(.rodata .rodata.*); } > FLASH
  .data : { *(.data .data.*); } > RAM AT > FLASH
  .bss (NOLOAD) : { *(.bss .bss.* COMMON); } > RAM
  /DISCARD/ : { *(.ARM.exidx .ARM.exidx.* .ARM.extab.*); }
}
