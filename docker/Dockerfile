ARG BASE_IMAGE=python:3.11-slim

# Stage 1: clone and install dependencies
FROM ${BASE_IMAGE} AS builder

ARG REPO_URL=https://github.com/NIyueeE/just-printing-server.git
ARG REF=main

RUN apt-get update && apt-get install -y --no-install-recommends git && \
    rm -rf /var/lib/apt/lists/*

RUN git clone --depth 1 --branch ${REF} ${REPO_URL} /app

WORKDIR /app
RUN pip install --no-cache-dir .

# Stage 2: minimal runtime image
FROM ${BASE_IMAGE}

COPY --from=builder /usr/local /usr/local
COPY --from=builder /app /app

WORKDIR /app

CMD ["python", "backend/main.py"]
