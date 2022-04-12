FROM archlinux:latest
RUN pacman -Syy
RUN pacman -S --noconfirm htop