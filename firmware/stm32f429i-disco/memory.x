/* STM32F429ZIT6 (STM32F429I-DISC1). The 8 MiB SDRAM (IS42S16400J, FMC bank 2 at 0xD0000000)
 * holds the two framebuffers; it is initialised at run time and not used by the linker. The
 * 64 KiB CCM RAM at 0x10000000 is not reachable by DMA2D/LTDC and is left unused. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 2048K
  RAM   : ORIGIN = 0x20000000, LENGTH = 192K
}
