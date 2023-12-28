ARG BUILD_FROM
FROM $BUILD_FROM

FROM ghcr.io/wez/pview-no-tls:latest as pview
COPY --from=pview /app/pview /app/pview
COPY run.sh /run.sh
CMD [ "/run.sh" ]
