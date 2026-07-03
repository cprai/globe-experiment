FROM mcr.microsoft.com/devcontainers/rust:1-bookworm

ENV CLAUDE_CONFIG_DIR=/home/vscode/.claude
ENV LIBGL_ALWAYS_SOFTWARE=1
ENV VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json:/usr/share/vulkan/icd.d/lvp_icd.aarch64.json

RUN apt-get update
RUN apt-get install -y mesa-vulkan-drivers libvulkan1 vulkan-tools libgl1-mesa-dri libglx-mesa0 mesa-utils
RUN apt-get install -y nodejs npm

RUN rustup component add --toolchain nightly-x86_64-unknown-linux-gnu rustfmt
RUN cargo install --git https://github.com/wgsl-analyzer/wgsl-analyzer wgslfmt
RUN cargo install --git https://github.com/gfx-rs/wgpu naga-cli

RUN npm install -g @anthropic-ai/claude-code
ENV DISABLE_AUTOUPDATER=1

RUN mkdir -p /home/vscode/.claude
RUN chown -R vscode:vscode /home/vscode/.claude
