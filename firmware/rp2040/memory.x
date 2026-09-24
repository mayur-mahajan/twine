/* RP2040 with 2 MiB QSPI flash (Raspberry Pi Pico): the 256-byte second-stage bootloader
   (boot2) at the start of flash, 264 KiB SRAM used as one block. Adjust FLASH for your board. */
MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    FLASH : ORIGIN = 0x10000100, LENGTH = 2048K - 0x100
    RAM   : ORIGIN = 0x20000000, LENGTH = 264K
}
