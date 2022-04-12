# Once you have built these images, you can bash into them with docker
# docker run -it example-arch:latest /bin/bash
.PHONY: build-images
build-images:
	docker build -f container-scanning/archlinux.latest.Dockerfile . -t example-arch:latest
	docker build -f container-scanning/debian.11.Dockerfile . -t example-debian:11
	docker build -f container-scanning/fedora.35.Dockerfile . -t example-fedora:35
	docker build -f container-scanning/oraclelinux.8.Dockerfile . -t example-oraclelinux:8
	docker build -f container-scanning/suse.bci-base.15_4.Dockerfile . -t example-suse:15.4
	docker build -f container-scanning/ubuntu.latest.Dockerfile . -t example-ubuntu:latest