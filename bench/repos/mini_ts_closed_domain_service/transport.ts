export enum DeliveryChannel {
  Email = "email",
  Sms = "sms",
}

const ROUTES: Partial<Record<DeliveryChannel, string>> = {
  [DeliveryChannel.Email]: "mailer",
};

export function routeChannel(channel: DeliveryChannel): string {
  return ROUTES[channel]!.toUpperCase().toLowerCase();
}
