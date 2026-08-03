data "hcloud_ssh_key" "operator" {
  name = var.ssh_key_name
}

resource "hcloud_firewall" "cache" {
  name = var.name

  dynamic "rule" {
    for_each = length(var.ssh_source_cidrs) == 0 ? [] : [1]
    content {
      direction  = "in"
      protocol   = "tcp"
      port       = "22"
      source_ips = var.ssh_source_cidrs
    }
  }

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "80"
    source_ips = ["0.0.0.0/0", "::/0"]
  }

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "443"
    source_ips = ["0.0.0.0/0", "::/0"]
  }

  rule {
    direction  = "in"
    protocol   = "icmp"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
}

resource "hcloud_server" "cache" {
  name         = var.name
  image        = "ubuntu-24.04"
  server_type  = var.server_type
  location     = var.location
  backups      = var.enable_backups
  ssh_keys     = [data.hcloud_ssh_key.operator.id]
  firewall_ids = [hcloud_firewall.cache.id]
  user_data    = file("${path.module}/cloud-init.yaml")

  public_net {
    ipv4_enabled = true
    ipv6_enabled = true
  }

  labels = {
    application = "mise-cache"
    managed-by  = "terraform"
  }

  lifecycle {
    ignore_changes = [ssh_keys]
  }
}

resource "minio_s3_bucket" "cache" {
  bucket         = var.object_storage_bucket
  acl            = "private"
  object_locking = false
}

resource "cloudflare_dns_record" "cache" {
  zone_id = var.cloudflare_zone_id
  name    = var.domain
  content = hcloud_server.cache.ipv4_address
  type    = "A"
  ttl     = 300
  proxied = false
  comment = "mise-cache origin managed by Terraform"
}
