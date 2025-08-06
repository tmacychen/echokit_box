#!/bin/bash

cd $HOME
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env

# https://narukara.github.io/rust-on-esp-book-zh-cn/installation/riscv-and-xtensa.html
cargo install --locked espup
espup install

cargo install --locked espflash
cargo install --locked ldproxy
cargo install --locked esp-generate
