/*
 * crt0.S — Minimal startup for rCore user-mode programs.
 *
 * Replaces picolibc's crt0 which does M-mode CSR accesses
 * (csrr mstatus, csrw fcsr) that are illegal in U-mode.
 *
 * The kernel's from_elf already loads ELF segment data at the
 * correct virtual addresses, so we only need to zero .bss.
 */

    .section .init, "ax"
    .globl _start
_start:
    /* Kernel already set sp for us */

    /* Initialize TLS: picolibc uses tp-relative access for errno etc. */
    la      tp, __tls_base

    /* Zero .bss: a0 = __bss_start, a1 = __bss_end */
    la      a0, __bss_start
    la      a1, __end
    bgeu    a0, a1, 2f
1:
    sd      zero, 0(a0)
    addi    a0, a0, 8
    bltu    a0, a1, 1b
2:

    /* Call main(argc=0, argv=NULL) */
    li      a0, 0
    li      a1, 0
    call    main

    /* exit(return_value) */
    li      a7, 93              /* SYS_EXIT */
    ecall

    /* unreachable */
    j       .
