FROM archlinux:base-devel

ARG REMOTE_USER
ARG GIT_AUTHOR_NAME
ARG GIT_AUTHOR_EMAIL

ENV LIBGL_ALWAYS_SOFTWARE=1

RUN pacman -Syu --noconfirm --needed \
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

RUN git config --global user.name "$GIT_AUTHOR_NAME"
RUN git config --global user.email "$GIT_AUTHOR_EMAIL"
RUN git config --global --add safe.directory "*"

RUN rustup toolchain install stable
RUN rustup component add --toolchain nightly rustfmt

RUN cargo install --git https://github.com/wgsl-analyzer/wgsl-analyzer wgslfmt
RUN cargo install --git https://github.com/gfx-rs/wgpu naga-cli

RUN uv tool install serena-agent
RUN serena init
RUN serena setup claude-code

RUN curl --proto "=https" --proto-redir "=https" --tlsv1.3 -fsSL https://raw.githubusercontent.com/DeusData/codebase-memory-mcp/main/install.sh | bash -s -- --skip-config
RUN codebase-memory-mcp install -y
