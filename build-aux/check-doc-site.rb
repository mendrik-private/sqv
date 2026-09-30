#!/usr/bin/env ruby
# frozen_string_literal: true

require "pathname"
require "set"
require "cgi"
require "uri"

site = ARGV.fetch(0) { abort "usage: #{$PROGRAM_NAME} SITE_DIRECTORY" }
root = Pathname(site).realpath
failures = []

Dir.glob(root.join("**", "*.html").to_s).sort.each do |html_path|
  html = Pathname(html_path)
  contents = html.read
  identifiers = contents.scan(/\bid=["']([^"']+)["']/).flatten.to_set

  contents.scan(/<(?:a|img|link|script)\b[^>]+\b(?:href|src)=["']([^"']+)["']/i).flatten.each do |raw_reference|
    reference = CGI.unescapeHTML(raw_reference)
    reference = URI::DEFAULT_PARSER.unescape(reference)
    next if reference.empty? || reference.match?(%r{\A(?://|https?:|mailto:|data:|javascript:)})
    next if reference == "#"

    path, anchor = reference.split("#", 2)
    path = path.to_s.split("?", 2).first.to_s
    if path.empty?
      failures << "#{html.relative_path_from(root)}: missing anchor ##{anchor}" if anchor && !identifiers.include?(anchor)
      next
    end

    target = if path.start_with?("/sqv/")
      root.join(path.delete_prefix("/sqv/")).cleanpath
    elsif path.start_with?("/")
      failures << "#{html.relative_path_from(root)}: path is outside the /sqv/ Pages site: #{reference}"
      next
    else
      html.dirname.join(path).cleanpath
    end
    target = target.expand_path
    unless target.to_s == root.to_s || target.to_s.start_with?("#{root}/")
      failures << "#{html.relative_path_from(root)}: local target escapes the generated site: #{reference}"
      next
    end
    unless target.file? || target.directory?
      failures << "#{html.relative_path_from(root)}: missing local target #{reference}"
      next
    end

    next unless anchor && target.extname == ".html"

    target_ids = target.read.scan(/\bid=["']([^"']+)["']/).flatten
    failures << "#{html.relative_path_from(root)}: missing anchor ##{anchor} in #{path}" unless target_ids.include?(anchor)
  end
end

abort "Documentation link check failed:\n#{failures.join("\n")}" unless failures.empty?

site_404 = root.join("404.html")
abort "Documentation link check failed: missing 404.html" unless site_404.file?
abort "Documentation link check failed: Pages site URL is not /sqv/" unless site_404.read.include?("<base href=\"/sqv/\">")
