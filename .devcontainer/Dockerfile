FROM archlinux:base-devel

ARG REMOTE_USER

ENV LIBGL_ALWAYS_SOFTWARE=1

RUN pacman -Sy --noconfirm --needed \
        git \
        curl \
        rustup \
        nodejs \
        npm \
        python \
        python-pip \
        python-uv \
        vulkan-icd-loader \
        vulkan-swrast \
        vulkan-tools \
        mesa \
        mesa-utils \
        libglvnd \
    && pacman -Scc --noconfirm

RUN npm install -g --allow-scripts=@anthropic-ai/claude-code @anthropic-ai/claude-code
ENV DISABLE_AUTOUPDATER=1

RUN useradd -m $REMOTE_USER
USER $REMOTE_USER
ENV HOME=/home/$REMOTE_USER
ENV PATH=$HOME/.local/bin:$HOME/.cargo/bin:$PATH

RUN git config --global --add safe.directory "*"

RUN rustup toolchain install stable
RUN rustup component add --toolchain nightly rustfmt

RUN cargo install --git https://github.com/wgsl-analyzer/wgsl-analyzer wgsl-analyzer wgslfmt
RUN cargo install naga-cli
RUN cargo install shader_language_server

RUN uv tool install ty
RUN uv tool install ruff

RUN uv tool install serena-agent
RUN serena init
RUN serena setup claude-code

RUN curl --proto "=https" --proto-redir "=https" --tlsv1.3 -fsSL https://raw.githubusercontent.com/DeusData/codebase-memory-mcp/main/install.sh | bash -s -- --skip-config
RUN codebase-memory-mcp install -y
