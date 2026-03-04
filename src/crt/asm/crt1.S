.text
.globl _start
.type _start, @function
_start:
  xor %rbp, %rbp
  lea main(%rip), %rdi
  mov (%rsp), %rsi
  lea 8(%rsp), %rdx
  lea 16(%rsp, %rsi, 8), %rcx
  andq $-16, %rsp
  call __libc_start_main
  hlt
.size _start, . - _start
