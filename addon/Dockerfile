ARG BUILD_FROM
FROM ghcr.io/wez/pview-no-tls:latest as pview
FROM $BUILD_FROM
COPY run.sh /run.sh
COPY --from=pview /lib/libgcc*.so* /lib/
COPY --from=pview /app/pview /pview
CMD [ "/run.sh" ]
