# Dockerfile for Scrapn - Instagram Scraping API

# ------------------------------------------------------------------------------
# Dependency Caching Stage - For faster rebuilds
# ------------------------------------------------------------------------------
FROM rust:slim AS deps

# Install dependencies for building
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev file && \
    rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy only files needed for dependency resolution
COPY Cargo.toml Cargo.lock ./

# Create empty src/main.rs to satisfy cargo
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs

# Build dependencies only
RUN cargo build --release
RUN rm -rf src

# ------------------------------------------------------------------------------
# Builder Stage - Builds the actual application
# ------------------------------------------------------------------------------
FROM deps AS builder

# Copy actual source code
COPY . .

# Build the real application for release
RUN echo "Building real application from source code..." && \
    ls -la && \
    cargo build --release && \
    ls -la target/release/ && \
    file target/release/scrapn

# ------------------------------------------------------------------------------
# Final Stage
# ------------------------------------------------------------------------------
FROM debian:bookworm-slim

# Install CA certificates for HTTPS requests and other required libraries
RUN apt-get update && \
    apt-get install -y ca-certificates libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -g 1000 scrapn && \
    useradd -u 1000 -g scrapn -s /bin/bash -m scrapn

# Set up application directory
WORKDIR /home/scrapn/app

# Copy configuration
COPY --chown=scrapn:scrapn App.toml ./App.toml

# Create a .env file for any environment variables
RUN touch .env && chown scrapn:scrapn .env

# Copy the built binary and ensure it's executable
COPY --from=builder --chown=scrapn:scrapn /app/target/release/scrapn ./scrapn
RUN chmod +x ./scrapn

# Create directory for logs with appropriate permissions
RUN mkdir -p logs && chown -R scrapn:scrapn /home/scrapn

# Switch to non-root user
USER scrapn

# Expose the server port
EXPOSE 8000


# Start the application directly
CMD ["./scrapn"]
